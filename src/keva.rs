//! Read the active identity keys from a Keva v1 repository without modifying it.
//!
//! Layout verified against Douyin 41.1.0's arm64 libkeva.so:
//! KevaBlockFile::{Init, GetBlockPointer, FetchStringUTF8},
//! KevaChunkFile::{Init, FetchBlockIndex, GetChunkOccupiedNum}, and
//! KevaPrivateChunkFile::Visit. Unreferenced block contents are never identities.

use std::collections::BTreeSet;

const INVALID: &str = "应用 Keva 身份记录损坏或格式不受支持，拒绝导入";
const CHANGED: &str = "无法读取稳定的应用 Keva 身份快照，请稍后重试";
const WIDTHS: [usize; 7] = [2, 1, 3, 2, 3, 2, 2];

// Deliberately omit Debug: callers must not print device identifiers on failure.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Identity {
    pub(crate) device_id: String,
    pub(crate) install_id: String,
}

/// Both files must repeat byte-for-byte before their cross-file references are
/// used. A concurrent write causes a bounded retry, never a mixed snapshot.
pub(crate) fn snapshot(
    mut read: impl FnMut() -> Result<(Vec<u8>, Vec<u8>), String>,
) -> Result<Identity, String> {
    for _ in 0..3 {
        let first = read()?;
        let second = read()?;
        if first == second {
            return parse(&first.0, &first.1).map_err(str::to_owned);
        }
    }
    Err(CHANGED.into())
}

fn word(bytes: &[u8], offset: usize) -> Result<u32, &'static str> {
    let end = offset.checked_add(4).ok_or(INVALID)?;
    let value: [u8; 4] = bytes
        .get(offset..end)
        .ok_or(INVALID)?
        .try_into()
        .map_err(|_| INVALID)?;
    Ok(u32::from_le_bytes(value))
}

fn end_of(start: usize, count: usize, width: usize, limit: usize) -> Result<usize, &'static str> {
    start
        .checked_add(count.checked_mul(width).ok_or(INVALID)?)
        .filter(|end| *end <= limit)
        .ok_or(INVALID)
}

struct Pool {
    start: usize,
    count: usize,
    width: usize,
}

struct Blocks<'a> {
    bytes: &'a [u8],
    pools: Vec<Pool>,
}

impl<'a> Blocks<'a> {
    fn new(bytes: &'a [u8]) -> Result<Self, &'static str> {
        if bytes.get(..8) != Some(b"keva-blk") || word(bytes, 8)? != 1 {
            return Err(INVALID);
        }
        let dimensions = word(bytes, 28)? as usize;
        if dimensions == 0 {
            return Err(INVALID);
        }
        let sizes = end_of(32, dimensions, 4, bytes.len())?;
        let mut start = end_of(sizes, dimensions, 4, bytes.len())?;
        let mut pools = Vec::new();
        for dimension in 0..=dimensions {
            let (count, width) = if dimension == dimensions {
                (word(bytes, 24)? as usize, word(bytes, 20)? as usize)
            } else {
                (
                    word(bytes, 32 + dimension * 4)? as usize,
                    word(bytes, sizes + dimension * 4)? as usize,
                )
            };
            // The native allocator groups block counts in sets of 64.
            if !count.is_multiple_of(64) || width < 4 {
                return Err(INVALID);
            }
            let end = end_of(start, count, width, bytes.len())?;
            pools.push(Pool {
                start,
                count,
                width,
            });
            start = end;
        }
        Ok(Self { bytes, pools })
    }

    fn string(&self, mut index: usize) -> Result<&'a str, &'static str> {
        for pool in &self.pools {
            if index >= pool.count {
                index -= pool.count;
                continue;
            }
            let offset = pool.start + index * pool.width;
            let header = word(self.bytes, offset)?;
            let length = (header & 0x00ff_ffff) as usize;
            if header & 0xff00_0000 != 0x8100_0000 || length > pool.width - 4 {
                return Err(INVALID);
            }
            return std::str::from_utf8(&self.bytes[offset + 4..offset + 4 + length])
                .map_err(|_| INVALID);
        }
        Err(INVALID)
    }
}

fn parse(chunks: &[u8], blocks: &[u8]) -> Result<Identity, &'static str> {
    if chunks.get(..8) != Some(b"keva-chk") || word(chunks, 8)? != 1 {
        return Err(INVALID);
    }
    let blocks = Blocks::new(blocks)?;
    let dimensions = word(chunks, 24)? as usize;
    if !(1..=3).contains(&dimensions) || chunks.len() < 40 {
        return Err(INVALID);
    }
    let mut keys = BTreeSet::new();
    let mut device_id = None;
    let mut install_id = None;
    let mut visit = |record: &[u8]| -> Result<(), &'static str> {
        let header = word(record, 0)?;
        if header & 0x8000_0000 == 0 {
            return Ok(());
        }
        let kind = ((header >> 24) & 15) as usize;
        let width = *WIDTHS.get(kind.wrapping_sub(1)).ok_or(INVALID)? * 4;
        if width > record.len() {
            return Err(INVALID);
        }
        let key = blocks.string((header & 0x00ff_ffff) as usize)?;
        if key.is_empty() || !keys.insert(key) {
            return Err(INVALID);
        }
        let destination = match key {
            "device_id" => &mut device_id,
            "install_id" => &mut install_id,
            _ => return Ok(()),
        };
        if kind != 6 {
            return Err(INVALID);
        }
        let reference = word(record, 4)?;
        // Identity values must be inline string blocks, never null or big-file
        // references. Reject unrecognized reference flags as well.
        if reference & 0xff00_0000 != 0x0100_0000 {
            return Err(INVALID);
        }
        let value = blocks.string((reference & 0x00ff_ffff) as usize)?;
        if value.is_empty()
            || value.len() > 24
            || !value.bytes().all(|byte| byte.is_ascii_digit())
            || !value.bytes().any(|byte| byte != b'0')
        {
            return Err(INVALID);
        }
        *destination = Some(value.to_owned());
        Ok(())
    };

    let mut position = 40;
    for dimension in 0..dimensions {
        let count = word(chunks, 28 + dimension * 4)? as usize;
        let width = (dimension + 1) * 4;
        let end = end_of(position, count, width, chunks.len())?;
        while position < end {
            visit(&chunks[position..position + width])?;
            position += width;
        }
    }
    let end = end_of(position, word(chunks, 20)? as usize, 4, chunks.len())?;
    while position < end {
        let kind = ((word(chunks, position)? >> 24) & 15) as usize;
        let width = WIDTHS.get(kind.wrapping_sub(1)).copied().unwrap_or(1) * 4;
        let next = end_of(position, 1, width, end)?;
        visit(&chunks[position..next])?;
        position = next;
    }
    Ok(Identity {
        device_id: device_id.ok_or(INVALID)?,
        install_id: install_id.ok_or(INVALID)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    struct Fixture {
        chunks: Vec<u8>,
        blocks: Vec<u8>,
    }

    impl Fixture {
        fn new() -> Self {
            let mut fixture = Self {
                chunks: vec![0; 4096],
                blocks: vec![0; 16384],
            };
            fixture.chunks[..8].copy_from_slice(b"keva-chk");
            for (offset, value) in [(8, 1), (20, 576), (24, 3), (28, 64), (32, 64), (36, 64)] {
                put(&mut fixture.chunks, offset, value);
            }
            fixture.blocks[..8].copy_from_slice(b"keva-blk");
            for (offset, value) in [
                (8, 1),
                (20, 64),
                (24, 192),
                (28, 2),
                (32, 64),
                (36, 64),
                (40, 16),
                (44, 32),
            ] {
                put(&mut fixture.blocks, offset, value);
            }
            fixture.string(0, "install_id");
            fixture.string(4, "device_id");
            fixture.string(74, "1234567890123456");
            fixture.string(78, "2345678901234567890");
            fixture.record(304, 0, 78);
            fixture.record(352, 4, 74);
            fixture
        }

        fn offset(index: usize) -> usize {
            match index {
                0..64 => 48 + index * 16,
                64..128 => 1072 + (index - 64) * 32,
                _ => 3120 + (index - 128) * 64,
            }
        }

        fn string(&mut self, index: usize, value: &str) {
            let offset = Self::offset(index);
            put(&mut self.blocks, offset, 0x8100_0000 | value.len() as u32);
            self.blocks[offset + 4..offset + 4 + value.len()].copy_from_slice(value.as_bytes());
        }

        fn record(&mut self, offset: usize, key: u32, value: u32) {
            put(&mut self.chunks, offset, 0x8600_0000 | key);
            put(&mut self.chunks, offset + 4, 0x0100_0000 | value);
        }

        fn pair(&self) -> (Vec<u8>, Vec<u8>) {
            (self.chunks.clone(), self.blocks.clone())
        }

        fn parse(&self) -> Result<Identity, &'static str> {
            parse(&self.chunks, &self.blocks)
        }
    }

    #[test]
    fn follows_active_keys_across_fixed_and_dynamic_pools() {
        let mut fixture = Fixture::new();
        let identity = fixture.parse().unwrap();
        assert_eq!(identity.device_id, "1234567890123456");
        assert_eq!(identity.install_id, "2345678901234567890");
        fixture.string(128, "3456789012345678901");
        fixture.record(1576, 0, 128);
        put(&mut fixture.chunks, 304, 0);
        assert_eq!(fixture.parse().unwrap().install_id, "3456789012345678901");
        // Move to a wider fixed chunk slot. Only the active record is relevant.
        put(&mut fixture.chunks, 1576, 0);
        put(&mut fixture.chunks, 1580, 0);
        fixture.record(808, 0, 128);
        assert_eq!(fixture.parse().unwrap().install_id, "3456789012345678901");
    }

    #[test]
    fn ignores_stale_blocks_and_rejects_wrong_or_missing_key_associations() {
        let mut fixture = Fixture::new();
        fixture.string(79, "9999999999999999999");
        fixture.string(5, "other_id");
        fixture.record(360, 5, 79);
        assert_eq!(fixture.parse().unwrap().install_id, "2345678901234567890");
        // Keeping the expected value in the file cannot replace its active key.
        put(&mut fixture.chunks, 304, 0);
        assert!(fixture.parse().is_err());
        fixture.string(0, "install_id_x");
        fixture.record(304, 0, 78);
        assert!(fixture.parse().is_err());
    }

    #[test]
    fn rejects_duplicate_active_keys_even_with_distinct_key_blocks() {
        for key in [0, 5] {
            let mut fixture = Fixture::new();
            fixture.string(5, "install_id");
            fixture.record(360, key, 78);
            assert!(fixture.parse().is_err());
        }
        let mut fixture = Fixture::new();
        fixture.record(360, 0, 78);
        put(&mut fixture.chunks, 360, 0x0600_0000);
        assert!(fixture.parse().is_ok());
    }

    #[test]
    fn rejects_unsupported_headers_and_out_of_bounds_pools() {
        for (chunk, offset, value) in [
            (true, 0, 0),
            (true, 8, 2),
            (true, 24, 0),
            (true, 24, 4),
            (true, 28, u32::MAX),
            (true, 20, u32::MAX),
            (false, 0, 0),
            (false, 8, 2),
            (false, 28, 0),
            (false, 28, u32::MAX),
            (false, 32, u32::MAX),
            (false, 24, u32::MAX),
            (false, 40, u32::MAX),
            (false, 20, 0),
            (false, 40, 0),
        ] {
            let mut fixture = Fixture::new();
            put(
                if chunk {
                    &mut fixture.chunks
                } else {
                    &mut fixture.blocks
                },
                offset,
                value,
            );
            assert!(fixture.parse().is_err(), "header offset {offset}");
        }
        for length in [0, 7, 31, 47, 4095] {
            let mut fixture = Fixture::new();
            fixture.blocks.truncate(length);
            assert!(fixture.parse().is_err());
        }
        for length in [0, 7, 23, 39, 1023] {
            let mut fixture = Fixture::new();
            fixture.chunks.truncate(length);
            assert!(fixture.parse().is_err());
        }
    }

    #[test]
    fn rejects_bad_records_references_and_string_bounds() {
        for (offset, value) in [
            (304, 0x8000_0000),
            (304, 0x8800_0000),
            (304, 0x8600_ffff),
            (304, 0x8100_0000),
            (308, 0x8100_004e),
            (308, 0x4100_004e),
            (308, 0x1100_004e),
            (308, 0x0200_004e),
            (308, 0x0100_ffff),
        ] {
            let mut fixture = Fixture::new();
            put(&mut fixture.chunks, offset, value);
            assert!(fixture.parse().is_err(), "record offset {offset}");
        }
        let mut fixture = Fixture::new();
        fixture.record(40, 0, 78); // Eight-byte record cannot fit a four-byte slot.
        assert!(fixture.parse().is_err());
        let mut fixture = Fixture::new();
        put(&mut fixture.chunks, 20, 1);
        fixture.record(1576, 0, 78); // Dynamic record would overrun the declared end.
        assert!(fixture.parse().is_err());
        for value in [0x0100_0013, 0x8100_001d, 0x81ff_ffff] {
            let mut fixture = Fixture::new();
            put(&mut fixture.blocks, Fixture::offset(78), value);
            assert!(fixture.parse().is_err());
        }
        let mut fixture = Fixture::new();
        fixture.blocks[Fixture::offset(0) + 4] = 0xff;
        assert!(fixture.parse().is_err());
    }

    #[test]
    fn rejects_empty_zero_nondecimal_and_oversized_identity_values() {
        for value in [
            "",
            "000000",
            "12a34",
            "+1234",
            "1234\n",
            "１２３４",
            "1234567890123456789012345",
        ] {
            for index in [74, 78] {
                let mut fixture = Fixture::new();
                fixture.string(index, value);
                let error = fixture.parse().err().unwrap();
                assert_eq!(error, INVALID);
            }
        }
    }

    #[test]
    fn snapshot_retries_changed_files_and_never_accepts_a_mixed_pair() {
        let original = Fixture::new();
        let mut updated = Fixture::new();
        updated.string(128, "3456789012345678901");
        updated.record(304, 0, 128);
        let mixed = (original.chunks.clone(), updated.blocks.clone());
        let mut reads = vec![mixed, updated.pair(), updated.pair(), updated.pair()].into_iter();
        let result = snapshot(|| Ok(reads.next().unwrap())).unwrap();
        assert_eq!(result.install_id, "3456789012345678901");
        assert!(reads.next().is_none());

        let mut changed = false;
        let error = snapshot(|| {
            changed = !changed;
            Ok(if changed {
                original.pair()
            } else {
                updated.pair()
            })
        })
        .err()
        .unwrap();
        assert_eq!(error, CHANGED);
    }

    #[test]
    fn snapshot_does_not_hide_read_failures_or_stable_corruption() {
        let mut reads = 0;
        let error = snapshot(|| {
            reads += 1;
            Err("fixture read failed".into())
        })
        .err()
        .unwrap();
        assert_eq!(error, "fixture read failed");
        assert_eq!(reads, 1);
        let mut fixture = Fixture::new();
        put(&mut fixture.chunks, 304, 0);
        assert_eq!(snapshot(|| Ok(fixture.pair())).err().unwrap(), INVALID);
    }
}
