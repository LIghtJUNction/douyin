//! Medusa v3 outer envelope with explicit application and request inputs.
//! No application ID, license or SDK session constants are selected here.

use super::{
    aes_v3::AesV3,
    envelope::{InputTooShort, key_hash, xmxor},
};
use base64::{Engine, engine::general_purpose::STANDARD};

/// Inputs needed to seal an already encoded Medusa v3 message.
pub struct SealInput<'a> {
    pub message: &'a [u8],
    pub sign_key: &'a [u8; 32],
    pub aes_key: [u8; 16],
    pub aes_iv: [u8; 16],
    pub magic: [u8; 16],
    pub flags: [u8; 8],
    pub query_digest: &'a [u8; 32],
    pub field13_head: u8,
    pub khronos: u32,
    pub nonce: u32,
    pub packed_nonce: u32,
}

/// Seal a message and return the Base64 wire value.
pub fn seal(input: &SealInput<'_>, sboxes: &[u8; 1024]) -> Result<String, InputTooShort> {
    let (hash, mask) = key_hash(input.sign_key, input.nonce);
    let mut transformed = Vec::with_capacity(8 + input.message.len());
    transformed.extend_from_slice(&input.flags);
    transformed.extend_from_slice(&xmxor(input.message, &hash)?);
    transformed.reverse();
    for (index, byte) in transformed.iter_mut().enumerate() {
        *byte ^= mask[!index & 3];
    }
    let check = 0x1800_0001
        | (u32::from(input.query_digest[0] & 63) << 14)
        | (u32::from(input.field13_head & 63) << 8);
    let mut packed = Vec::with_capacity(11 + transformed.len());
    packed.push(0x35);
    packed.extend_from_slice(&input.packed_nonce.to_le_bytes());
    packed.extend_from_slice(&check.to_le_bytes());
    packed.extend_from_slice(&transformed);
    packed.extend_from_slice(&input.nonce.to_le_bytes()[2..]);
    transformed.fill(0);
    let encrypted = AesV3::new(input.aes_key, input.khronos, sboxes).encrypt(&packed, input.aes_iv);
    packed.fill(0);

    let mut output = Vec::with_capacity(24 + encrypted.len());
    output.extend_from_slice(&(3 ^ input.khronos).to_le_bytes());
    for (index, byte) in input.magic.iter().enumerate() {
        output.push(byte ^ input.khronos.to_le_bytes()[index & 3]);
    }
    output.extend_from_slice(&input.nonce.to_le_bytes()[..2]);
    output.extend_from_slice(&[0, 1]);
    output.extend_from_slice(&encrypted);
    Ok(STANDARD.encode(output))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_envelope_matches_all_four_public_reference_vectors() {
        for (variant, expected) in [
            "03f15365f7190c9fd7268f5ed6db9b15573e327d785600012f158963552303058338084ca5660f9ca9d7781f669f7d44f159c9e02e8e4237bbb01cebe563e27a2d489d4dbf8be3c7b73cc4a1a7c7bd6c880be185cd444524a42ee0aaab289c0b02d134ddd074a113498a92e90b9e837d16df61ad3cf3d91e794b243fcba65a5a5467ae81c8b6fffedfd8fffe4e903613",
            "02f15365f6190c9fd6268f5ed7db9b15563e327d78560001771589634523030583384848a5660d9d29f7781f76977f45f15989e43e8e42363bb01cebe563e27bad689d49bf8be1c6371cc4a5a7cfbd6c082be185cd444525a40ea0aebb209e0b02f134d9d07ca113498a92e90b96817d96ff61ad2cf3d91f794b243bcba6585b5467ae81d8befffe5ff8fffe5e983412",
            "01f15365f5190c9fd5268f5ed4db9b15553e327d78560001df15c963452b010403384848b56e0d9da9d7381b66977d44f179c9e43e8e40363bb01cefe56be27a2d48dd49af83e3c6371cc4a1b7c7bd6d880be181cd444725242ee0aebb209e0a82f174ddd07ca313c9aad2ed0b9e817c16df21ad2cf3d91ef96b243bcba65a5bd467ae81d8befdfe5fd8bffa4e903412",
            "00f15365f4190c9fd4268f5ed5db9b15543e327d785600013d15c963452b01050338484cb56e0f9c29d7781f669f7d45f15989e02e8642373b905cebe56be07a2d689d49af83e3c7b71c84a5a7cfbf6d882ba181dd444725242ee0aebb289e0b82d134ddc07ca112c9aad2ed0b96817d16df21a92cf3d91e794b643fcbae5a5b5447ee81d8b6ffffdfd8fffa5e983613",
        ]
        .into_iter()
        .enumerate()
        {
            assert_eq!(reference(1_700_000_000 + variant as u32), expected);
        }
    }

    fn reference(khronos: u32) -> String {
        let decoded = STANDARD
            .decode(include_str!("../../assets/metasec-fixtures/aes-v3-sbox.base64").trim())
            .unwrap();
        let sboxes: &[u8; 1024] = decoded.as_slice().try_into().unwrap();
        let message: Vec<u8> = (0..100).collect();
        let sign_key: [u8; 32] = std::array::from_fn(|index| index as u8);
        let query_digest = [
            0xcc, 0xac, 0x73, 0x5c, 0x4f, 0x62, 0xfd, 0xe1, 0xf3, 0x30, 0x56, 0x5c, 0xe9, 0x14,
            0x39, 0xe0, 0xb7, 0x35, 0xa7, 0x9d, 0x07, 0x0b, 0x13, 0x9a, 0xce, 0xf1, 0x34, 0x98,
            0xda, 0x31, 0xf1, 0xaa,
        ];
        let field13_head = 0x45;
        let input = SealInput {
            message: &message,
            sign_key: &sign_key,
            aes_key: [
                0xf1, 0x59, 0x33, 0x76, 0x76, 0x6e, 0xa9, 0x8d, 0x34, 0xf3, 0x1b, 0x05, 0x7a, 0x9d,
                0x5b, 0xe4,
            ],
            aes_iv: [
                0x1f, 0xe1, 0x09, 0xa4, 0x12, 0x52, 0x83, 0xf4, 0x18, 0xde, 0x9e, 0x05, 0x1a, 0x96,
                0x9e, 0x12,
            ],
            magic: [
                0xf7, 0xe8, 0x5f, 0xfa, 0xd7, 0xd7, 0xdc, 0x3b, 0xd6, 0x2a, 0xc8, 0x70, 0x57, 0xcf,
                0x61, 0x18,
            ],
            flags: [0x40, 1, 0, 0, 0, 0, 0, 0],
            query_digest: &query_digest,
            field13_head,
            khronos,
            nonce: 0x1234_5678,
            packed_nonce: 0x2345_6789,
        };
        hex(&STANDARD.decode(seal(&input, sboxes).unwrap()).unwrap())
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}
