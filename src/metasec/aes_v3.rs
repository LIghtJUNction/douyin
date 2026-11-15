//! AES variant recovered from the Medusa v3 envelope.
//! The four S-box windows and permutations are selected by Khronos.

/// A keyed AES v3 instance.
pub struct AesV3 {
    sbox: [u8; 256],
    columns: [usize; 4],
    order: [usize; 16],
    keys: [[u8; 4]; 12],
}

impl AesV3 {
    /// Expand a 128-bit key using the selected 1024-byte S-box table.
    #[must_use]
    pub fn new(key: [u8; 16], khronos: u32, sboxes: &[u8; 1024]) -> Self {
        const COLUMNS: [[usize; 4]; 4] = [[1, 0, 2, 3], [2, 0, 3, 1], [0, 1, 3, 2], [1, 0, 2, 3]];
        const ORDERS: [[usize; 16]; 4] = [
            [0, 9, 14, 11, 4, 13, 2, 7, 8, 1, 6, 15, 12, 5, 10, 3],
            [0, 9, 14, 15, 4, 13, 2, 7, 8, 1, 6, 3, 12, 5, 10, 11],
            [0, 9, 14, 7, 4, 13, 2, 11, 8, 1, 6, 3, 12, 5, 10, 15],
            [0, 9, 14, 11, 4, 13, 2, 7, 8, 1, 6, 15, 12, 5, 10, 3],
        ];
        const INITIAL: [u32; 4] = [0xca02_5ddc, 0x823d_c546, 0xc942_0583, 0xc298_225f];
        let variant = (khronos & 3) as usize;
        let sbox: [u8; 256] = sboxes[variant * 256..variant * 256 + 256]
            .try_into()
            .expect("each AES v3 variant has a complete S-box");
        let columns = COLUMNS[variant];
        let order = ORDERS[variant];
        let initial = INITIAL[variant];
        let initial_bytes = initial.to_le_bytes();
        let mut expanded = [0; 48];
        for index in 0..16 {
            expanded[index] = initial_bytes[index & 3] ^ key[index];
        }
        let mut round = 8_u32;
        for index in 4..12 {
            let at = 4 * (index - 1);
            let mut a = expanded[at];
            let mut b = expanded[at + 1];
            let mut c = expanded[at + 2];
            let mut d = expanded[at + 3];
            if index & 3 == 0 {
                let replacement = ((initial >> (round & 24)) ^ u32::from(sbox[b as usize])) as u8;
                b = sbox[c as usize];
                c = sbox[d as usize];
                d = sbox[a as usize];
                a = replacement;
            }
            round += 2;
            expanded[at + 4] = a ^ expanded[at - 12];
            expanded[at + 5] = b ^ expanded[at - 11];
            expanded[at + 6] = c ^ expanded[at - 10];
            expanded[at + 7] = d ^ expanded[at - 9];
        }
        let mut keys = [[0; 4]; 12];
        for (index, key) in keys.iter_mut().enumerate() {
            key.copy_from_slice(&expanded[index * 4..index * 4 + 4]);
        }
        expanded.fill(0);
        Self {
            sbox,
            columns,
            order,
            keys,
        }
    }

    /// Encrypt the sparse representation and inject its key bits into the input.
    #[must_use]
    pub fn encrypt(&self, data: &[u8], iv: [u8; 16]) -> Vec<u8> {
        let source = |index: usize| data.get(index).copied().unwrap_or(0);
        let mut plaintext = [0; 32];
        for (index, byte) in plaintext[..31].iter_mut().enumerate() {
            let at = index * 8;
            *byte = ((source(at) >> 4) & 2)
                | (source(at + 1) & 64)
                | ((source(at + 2) >> 2) & 1)
                | ((source(at + 3) << 3) & 128)
                | ((source(at + 4) >> 1) & 4)
                | ((source(at + 5) << 3) & 16)
                | ((source(at + 6) << 5) & 32)
                | ((source(at + 7) >> 4) & 8);
        }
        plaintext[31] = 1;
        let mut key = [0; 32];
        let mut previous = iv;
        for (index, chunk) in plaintext.as_chunks::<16>().0.iter().enumerate() {
            let mut block = *chunk;
            for (byte, mask) in block.iter_mut().zip(previous) {
                *byte ^= mask;
            }
            let encrypted = self.block(&block);
            key[index * 16..index * 16 + 16].copy_from_slice(&encrypted);
            previous = encrypted;
        }
        plaintext.fill(0);
        let mut output = data.to_vec();
        for (index, key_byte) in key[..31].iter().enumerate() {
            let at = index * 8;
            let replacements = [
                (at, 0xdf, (key_byte << 4) & 32),
                (at + 1, 0xbf, key_byte & 64),
                (at + 2, 0xfb, (key_byte << 2) & 4),
                (at + 3, 0xef, (key_byte >> 3) & 16),
                (at + 4, 0xf7, key_byte.wrapping_add(*key_byte) & 8),
                (at + 5, 0xfd, (key_byte >> 3) & 2),
                (at + 6, 0xfe, (key_byte >> 5) & 1),
                (at + 7, 0x7f, (key_byte << 4) & 128),
            ];
            for (position, mask, bits) in replacements {
                if let Some(byte) = output.get_mut(position) {
                    *byte = (*byte & mask) | bits;
                }
            }
        }
        let mut result = Vec::with_capacity(1 + output.len());
        result.push(key[31]);
        result.extend_from_slice(&output);
        result
    }

    fn block(&self, input: &[u8; 16]) -> [u8; 16] {
        let mut rows = [[0; 4]; 4];
        for (row, chunk) in rows.iter_mut().zip(input.as_chunks::<4>().0) {
            row.copy_from_slice(chunk);
        }
        self.add_columns(&mut rows, &self.sub_keys(0));
        for round in 1..3 {
            self.substitute(&mut rows);
            self.shuffle(&mut rows);
            if round == 1 {
                self.shuffle_columns(&mut rows);
                mix(&mut rows);
            }
            self.add_columns(&mut rows, &self.sub_keys(round * 4));
        }
        self.add(&mut rows, &self.sub_keys(4));
        let mut output = [0; 16];
        for (chunk, row) in output.as_chunks_mut::<4>().0.iter_mut().zip(rows) {
            chunk.copy_from_slice(&row);
        }
        output
    }

    fn sub_keys(&self, from: usize) -> [[u8; 4]; 4] {
        let mut result = [[0; 4]; 4];
        for (index, key) in result.iter_mut().enumerate() {
            *key = self.keys[from + index];
        }
        result
    }

    fn add(&self, rows: &mut [[u8; 4]; 4], keys: &[[u8; 4]; 4]) {
        for (row, key) in rows.iter_mut().zip(keys) {
            for (byte, mask) in row.iter_mut().zip(key) {
                *byte ^= mask;
            }
        }
    }

    fn add_columns(&self, rows: &mut [[u8; 4]; 4], keys: &[[u8; 4]; 4]) {
        for (row, key) in rows.iter_mut().zip(keys) {
            for (column, source) in self.columns.iter().enumerate() {
                row[column] ^= key[*source];
            }
        }
    }

    fn substitute(&self, rows: &mut [[u8; 4]; 4]) {
        for row in rows.iter_mut() {
            for byte in row.iter_mut() {
                *byte = self.sbox[usize::from(*byte)];
            }
        }
        let old = *rows;
        for (row, source) in rows.iter_mut().zip(self.columns) {
            *row = old[source];
        }
    }

    fn shuffle(&self, rows: &mut [[u8; 4]; 4]) {
        let mut flat = [0; 16];
        for (chunk, row) in flat.as_chunks_mut::<4>().0.iter_mut().zip(rows.iter()) {
            chunk.copy_from_slice(row);
        }
        let mut shuffled = [0; 16];
        for (index, byte) in shuffled.iter_mut().enumerate() {
            *byte = flat[self.order[index]];
        }
        for (row, chunk) in rows.iter_mut().zip(shuffled.as_chunks::<4>().0) {
            row.copy_from_slice(chunk);
        }
    }

    fn shuffle_columns(&self, rows: &mut [[u8; 4]; 4]) {
        let old = *rows;
        for (row, source) in rows.iter_mut().zip(old) {
            for (column, index) in self.columns.iter().enumerate() {
                row[column] = source[*index];
            }
        }
    }
}

impl Drop for AesV3 {
    fn drop(&mut self) {
        self.keys.fill([0; 4]);
        self.sbox.fill(0);
    }
}

fn mix(rows: &mut [[u8; 4]; 4]) {
    for column in 0..4 {
        let total = rows[0][column] ^ rows[1][column] ^ rows[2][column] ^ rows[3][column];
        let first = rows[0][column];
        rows[0][column] ^= total ^ xtime(rows[0][column] ^ rows[1][column]);
        rows[1][column] ^= total ^ xtime(rows[1][column] ^ rows[2][column]);
        rows[2][column] ^= total ^ xtime(rows[2][column] ^ rows[3][column]);
        rows[3][column] ^= total ^ xtime(rows[3][column] ^ first);
    }
}

fn xtime(value: u8) -> u8 {
    (value << 1) ^ if value & 128 == 0 { 0 } else { 0x1b }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{Engine, engine::general_purpose::STANDARD};

    #[test]
    fn all_variants_match_public_key_and_block_vectors() {
        let decoded = STANDARD
            .decode(include_str!("../../assets/metasec-fixtures/aes-v3-sbox.base64").trim())
            .unwrap();
        let sboxes: &[u8; 1024] = decoded.as_slice().try_into().unwrap();
        for (variant, (expected_keys, expected_block)) in [
            (
                "dc5c00c9d85804cdd45408c1d0500cc56e64442cb63c40e162684820b23844e5f2b84e8d44840e6c26ec464c94d402a9",
                "691b49f4c90322bcaba44d4642ff53ba",
            ),
            (
                "46c43f8142c03b854ecc37894ac8338df37df266b1bdc9e3ff71fe6ab5b9cde709456911b8f8a0f247895e98f230937f",
                "178cbcbcf359ed25c7fc1c0a3ba4db9c",
            ),
            (
                "830440ca870044ce8b0c48c28f084cc6c3ff457044ff01becff3497c40fb05ba5515957811ea94c6de19ddba9ee2d800",
                "8ec5a2086fdb9d8876478056d0b419a1",
            ),
            (
                "5f239ac15b279ec5572b92c9532f96cddd5e878786791942d1528b8b827d1d466af65743ec8f4e013dddc58abfa0d8cc",
                "19e63c90ada97c72ab61c05bab01f2e6",
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let key: [u8; 16] = std::array::from_fn(|index| index as u8);
            let input: [u8; 16] = std::array::from_fn(|index| index as u8);
            let cipher = AesV3::new(key, 1_700_000_000 + variant as u32, sboxes);
            assert_eq!(hex(&cipher.keys.concat()), expected_keys);
            assert_eq!(hex(&cipher.block(&input)), expected_block);
        }
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}
