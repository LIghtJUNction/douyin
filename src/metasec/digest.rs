//! Arithmetic recovered from the pinned Metasec module at `0x4420f0`.
//! These primitives do not add SDK-dependent state bits or sign requests.

use base64::{Engine, engine::general_purpose::STANDARD};
use std::{error::Error, fmt};

const INITIAL_STATE: [u32; 4] = [0x79e0f2fb, 0xc8b52570, 0xebc2f8cd, 0x7c104d93];
const FINAL_XOR: [u32; 4] = [0x19be4866, 0xe85986b4, 0xe19b326e, 0x71d1d7d4];
const PADDING: [u8; 64] = [
    0x84, 0x96, 0x77, 0x9d, 0xd4, 0x15, 0x0b, 0xf8, 0xdb, 0x6d, 0xbc, 0xb6, 0xd7, 0x61, 0xe9, 0x3e,
    0xb5, 0x04, 0x2b, 0xba, 0xbb, 0x5b, 0x5d, 0xaf, 0xe1, 0xba, 0xee, 0xe3, 0xda, 0x21, 0x16, 0x06,
    0x92, 0x4b, 0x1f, 0x0e, 0x7b, 0x17, 0x31, 0x11, 0xbb, 0xa7, 0x8c, 0xfe, 0xc0, 0xa0, 0x6d, 0x6c,
    0x85, 0xf8, 0x26, 0xe1, 0x29, 0x0e, 0x6a, 0xd4, 0xf3, 0x02, 0x20, 0xaa, 0xc5, 0x7f, 0xf2, 0xfd,
];
const ROUND_CONSTANTS: [u32; 64] = [
    2813263392, 1900671446, 1206176263, 3920990055, 2478384350, 3323252981, 951050479, 2998644083,
    3796124867, 2637884571, 3749394404, 3635506496, 4172034488, 1991878923, 836252654, 2503986788,
    1785408328, 2350088822, 1831116816, 416944715, 3152773870, 972548694, 345984993, 580470460,
    52949769, 4109700710, 1658153386, 2017566610, 1303414951, 1949343768, 4078799092, 793695825,
    2601778777, 1170575629, 927723756, 3584407198, 4021086998, 1901021201, 1285038907, 3387353009,
    3496098623, 3747499887, 3434287684, 2916792406, 1873779870, 1652575760, 1873273501, 2118294865,
    3451067905, 3006915609, 3835144355, 3958154940, 390516055, 4227134794, 3588893323, 3035639506,
    85111186, 4021112105, 3583765509, 705967842, 4209669057, 515750632, 2415024061, 760821021,
];
const MESSAGE_SCHEDULE: [usize; 64] = [
    0, 14, 13, 12, 11, 10, 9, 7, 8, 6, 5, 4, 3, 2, 1, 15, 0, 5, 10, 1, 12, 11, 15, 2, 9, 14, 3, 8,
    6, 7, 4, 13, 8, 5, 12, 14, 1, 12, 7, 13, 10, 0, 2, 6, 4, 11, 15, 3, 8, 6, 15, 5, 13, 9, 10, 2,
    2, 14, 7, 12, 4, 1, 11, 3,
];
const LEFT_ROTATIONS: [u32; 64] = [
    7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9,
    14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15,
    21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
];

/// Apply the recovered 64-round compression function to one block.
#[must_use]
pub fn compress_block(state: [u32; 4], block: &[u8; 64]) -> [u32; 4] {
    let [mut a, mut b, mut c, mut d] = state;
    for round in 0..64 {
        let mix = match round {
            0..=15 => (b & c) | (!b & d),
            16..=31 => (d & b) | (!d & c),
            32..=47 => b ^ c ^ d,
            _ => c ^ (b | !d),
        };
        let offset = MESSAGE_SCHEDULE[round] * 4;
        let word = u32::from_le_bytes(
            block[offset..offset + 4]
                .try_into()
                .expect("the message schedule selects four bytes inside a block"),
        );
        let sum = a
            .wrapping_add(mix)
            .wrapping_add(ROUND_CONSTANTS[round])
            .wrapping_add(word);
        let next = b.wrapping_add(sum.rotate_left(LEFT_ROTATIONS[round]));
        a = d;
        d = c;
        c = b;
        b = next;
    }
    [
        a.wrapping_add(state[0]),
        b.wrapping_add(state[1]),
        c.wrapping_add(state[2]),
        d.wrapping_add(state[3]),
    ]
}

/// Error returned when data is added after finalization.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DigestFinalized;

impl fmt::Display for DigestFinalized {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("digest has already been finalized")
    }
}

impl Error for DigestFinalized {}

/// Incremental form of the recovered 16-byte Medusa digest.
#[derive(Clone)]
pub struct MedusaDigest {
    state: [u32; 4],
    pending: [u8; 64],
    pending_length: usize,
    byte_count: u64,
    result: Option<[u8; 16]>,
}

impl Default for MedusaDigest {
    fn default() -> Self {
        Self {
            state: INITIAL_STATE,
            pending: [0; 64],
            pending_length: 0,
            byte_count: 0,
            result: None,
        }
    }
}

impl MedusaDigest {
    /// Construct an empty digest context.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add bytes, retaining a partial block until enough data arrives.
    pub fn update(&mut self, mut input: &[u8]) -> Result<(), DigestFinalized> {
        if self.result.is_some() {
            return Err(DigestFinalized);
        }
        self.byte_count = self.byte_count.wrapping_add(input.len() as u64);
        if self.pending_length != 0 {
            let take = (64 - self.pending_length).min(input.len());
            self.pending[self.pending_length..self.pending_length + take]
                .copy_from_slice(&input[..take]);
            self.pending_length += take;
            input = &input[take..];
            if self.pending_length == 64 {
                self.state = compress_block(self.state, &self.pending);
                self.pending_length = 0;
            }
        }
        while input.len() >= 64 {
            let block: &[u8; 64] = input[..64]
                .try_into()
                .expect("the loop condition guarantees a complete block");
            self.state = compress_block(self.state, block);
            input = &input[64..];
        }
        if !input.is_empty() {
            self.pending[..input.len()].copy_from_slice(input);
            self.pending_length = input.len();
        }
        Ok(())
    }

    /// Finish the digest; repeated calls return the same bytes.
    #[must_use]
    pub fn finalize(&mut self) -> [u8; 16] {
        if let Some(result) = self.result {
            return result;
        }
        let pad_length = if self.pending_length < 60 {
            60 - self.pending_length
        } else {
            124 - self.pending_length
        };
        let mut tail = Vec::with_capacity(self.pending_length + pad_length + 4);
        tail.extend_from_slice(&self.pending[..self.pending_length]);
        tail.extend_from_slice(&PADDING[..pad_length]);
        let bit_length = (self.byte_count.wrapping_mul(8) & 0xffff_ffff) as u32;
        tail.extend_from_slice(&bit_length.to_le_bytes());
        for block in tail.as_chunks::<64>().0 {
            self.state = compress_block(self.state, block);
        }
        tail.fill(0);
        self.pending.fill(0);
        let mut result = [0; 16];
        for (index, word) in self.state.iter().enumerate() {
            result[index * 4..index * 4 + 4]
                .copy_from_slice(&(word ^ FINAL_XOR[index]).to_le_bytes());
        }
        self.result = Some(result);
        result
    }

    /// Compute the 16-byte digest in one call.
    #[must_use]
    pub fn digest(input: &[u8]) -> [u8; 16] {
        let mut digest = Self::new();
        digest
            .update(input)
            .expect("a new digest context cannot be finalized");
        digest.finalize()
    }
}

/// The two recovered checksum branches, selected by the caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TailLength {
    Eight,
    Twelve,
}

/// Append the recovered four-byte checksum and select the branch bit.
#[must_use]
pub fn append_tail(digest: &[u8; 16], length: TailLength) -> [u8; 20] {
    let count = match length {
        TailLength::Eight => 8,
        TailLength::Twelve => 12,
    };
    let mut checksum = 0x2022_0420_u32;
    for (index, byte) in digest[..count].iter().enumerate() {
        checksum = if index & 1 == 1 {
            !((checksum ^ (checksum >> 5)) ^ ((checksum << 11) | u32::from(*byte)))
        } else {
            checksum ^ (checksum << 7) ^ (checksum >> 3) ^ u32::from(*byte)
        };
    }
    let mut result = [0; 20];
    result[..16].copy_from_slice(digest);
    result[16..].copy_from_slice(&checksum.to_le_bytes());
    if count == 12 {
        result[16] |= 4;
    } else {
        result[16] &= 0xfb;
    }
    result
}

/// Encode the little-endian time word used by the recovered Argus path.
#[must_use]
pub fn encode_argus_time_word(time: u32) -> String {
    STANDARD.encode(time.to_le_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patterned(length: usize) -> Vec<u8> {
        (0..length)
            .map(|index| (index * 131 + 0x80) as u8)
            .collect()
    }

    #[test]
    fn digest_matches_public_reference_vectors_at_padding_boundaries() {
        for (input, expected) in [
            (vec![], "ce206872fff5963f54efdfd3145f8ea8"),
            (b"abc".to_vec(), "c43d0f3758d4beb30a3fbf8f4036e574"),
            (patterned(60), "93ddc33cf8951ff559f817a63a8bd6f2"),
            (patterned(64), "1303495a1078b779ff87fd40ee1a7ad0"),
            (patterned(65), "329d59557dcb53e918602b43b0e96500"),
        ] {
            assert_eq!(hex(&MedusaDigest::digest(&input)), expected);
        }
    }

    #[test]
    fn incremental_updates_match_one_shot_and_finalization_is_idempotent() {
        let input = patterned(320);
        for size in [1, 3, 59, 60, 61, 63, 64, 65, 127, 128, 129] {
            let mut digest = MedusaDigest::new();
            for chunk in input.chunks(size) {
                digest.update(chunk).unwrap();
                digest.update(&[]).unwrap();
            }
            assert_eq!(digest.finalize(), MedusaDigest::digest(&input));
            assert_eq!(digest.finalize(), MedusaDigest::digest(&input));
            assert_eq!(digest.update(&[]), Err(DigestFinalized));
        }
    }

    #[test]
    fn tail_and_argus_encoding_match_public_reference_vectors() {
        let input: [u8; 16] = std::array::from_fn(|index| index as u8);
        assert_eq!(
            hex(&append_tail(&input, TailLength::Eight)),
            "000102030405060708090a0b0c0d0e0f020b2e3e"
        );
        assert_eq!(
            hex(&append_tail(&input, TailLength::Twelve)),
            "000102030405060708090a0b0c0d0e0f5d9a37d8"
        );
        assert_eq!(encode_argus_time_word(0), "AAAAAA==");
        assert_eq!(encode_argus_time_word(0x04030201), "AQIDBA==");
        assert_eq!(encode_argus_time_word(u32::MAX), "/////w==");
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}
