//! Speck-128/256 operations used by the recovered Helios module.
//! This component does not select an SDK key or construct a complete signature.

/// A Speck-128/256 key expanded into 34 round keys.
pub struct SpeckKey {
    round_keys: [u64; 34],
}

impl SpeckKey {
    /// Expand a 256-bit key in the recovered little-endian layout.
    #[must_use]
    pub fn new(key: &[u8; 32]) -> Self {
        let word = |offset: usize| {
            u64::from_le_bytes(
                key[offset..offset + 8]
                    .try_into()
                    .expect("each key word contains eight bytes"),
            )
        };
        let mut a = word(0);
        let mut queue = [0; 36];
        queue[..3].copy_from_slice(&[word(8), word(16), word(24)]);
        let mut round_keys = [0; 34];
        round_keys[0] = a;
        for round in 0..33 {
            let next = queue[round].rotate_right(8).wrapping_add(a) ^ round as u64;
            queue[round + 3] = next;
            a = a.rotate_right(61) ^ next;
            round_keys[round + 1] = a;
        }
        queue.fill(0);
        Self { round_keys }
    }

    /// Encrypt one 128-bit block.
    #[must_use]
    pub fn encrypt_block(&self, block: &[u8; 16]) -> [u8; 16] {
        let mut y = u64::from_le_bytes(
            block[..8]
                .try_into()
                .expect("the first block word contains eight bytes"),
        );
        let mut x = u64::from_le_bytes(
            block[8..]
                .try_into()
                .expect("the second block word contains eight bytes"),
        );
        for key in self.round_keys {
            x = x.rotate_right(8).wrapping_add(y) ^ key;
            y = y.rotate_right(61) ^ x;
        }
        let mut result = [0; 16];
        result[..8].copy_from_slice(&y.to_le_bytes());
        result[8..].copy_from_slice(&x.to_le_bytes());
        result
    }

    /// Apply PKCS#7 padding and encrypt independent blocks.
    #[must_use]
    pub fn encrypt_padded(&self, input: &[u8]) -> Vec<u8> {
        let padding = 16 - input.len() % 16;
        let mut padded = Vec::with_capacity(input.len() + padding);
        padded.extend_from_slice(input);
        padded.resize(input.len() + padding, padding as u8);
        let mut result = Vec::with_capacity(padded.len());
        for block in padded.as_chunks::<16>().0 {
            result.extend_from_slice(&self.encrypt_block(block));
        }
        padded.fill(0);
        result
    }
}

impl Drop for SpeckKey {
    fn drop(&mut self) {
        self.round_keys.fill(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_cipher_authors_published_speck_128_256_vector() {
        let key: [u8; 32] = std::array::from_fn(|index| index as u8);
        let plain: [u8; 16] = [
            0x70, 0x6f, 0x6f, 0x6e, 0x65, 0x72, 0x2e, 0x20, 0x49, 0x6e, 0x20, 0x74, 0x68, 0x6f,
            0x73, 0x65,
        ];
        let key = SpeckKey::new(&key);
        assert_eq!(
            key.round_keys[..8],
            [
                0x0706050403020100,
                0x37253b31171d0309,
                0xfe1588ce93d80d52,
                0xe698e09f31334dfe,
                0xdb60f14bcbd834fd,
                0x2dafa7c34cc2c2f8,
                0xfbb8e2705e64a1db,
                0xdb6f99e4e383eaef,
            ]
        );
        assert_eq!(
            key.encrypt_block(&plain),
            [
                0x43, 0x8f, 0x18, 0x9c, 0x8d, 0xb4, 0xee, 0x4e, 0x3e, 0xf5, 0xc0, 0x05, 0x04, 0x01,
                0x09, 0x41,
            ]
        );
    }

    #[test]
    fn padding_adds_a_complete_block_at_aligned_lengths() {
        let key = SpeckKey::new(&[0xff; 32]);
        for length in [0, 1, 15, 16, 17, 31, 32, 33, 63, 64] {
            let input = vec![0x5a; length];
            let padding = 16 - length % 16;
            let encrypted = key.encrypt_padded(&input);
            assert_eq!(encrypted.len(), length + padding);
            for (index, block) in encrypted.as_chunks::<16>().0.iter().enumerate() {
                let mut expected = [padding as u8; 16];
                let offset = index * 16;
                for (position, byte) in expected.iter_mut().enumerate() {
                    if offset + position < length {
                        *byte = 0x5a;
                    }
                }
                assert_eq!(block, &key.encrypt_block(&expected));
            }
        }
    }
}
