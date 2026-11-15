//! X-Gorgon value construction with an explicit SDK version word.
//! Query bytes, body bytes and timestamp must match the eventual request.

use md5::{Digest, Md5};

/// Construct the recovered Gorgon wire value for a 16-bit request nonce.
#[must_use]
pub fn create(query: &[u8], body: Option<&[u8]>, khronos: u32, nonce: u16, sdk: u32) -> String {
    let query_hash = Md5::digest(query);
    let body_hash = body.filter(|value| !value.is_empty()).map(Md5::digest);
    let mut input = Vec::with_capacity(20);
    input.extend_from_slice(&query_hash[..4]);
    if let Some(hash) = body_hash {
        input.extend_from_slice(&hash[..4]);
    } else {
        input.extend_from_slice(&[0; 4]);
    }
    input.extend_from_slice(&[0; 4]);
    input.extend_from_slice(&sdk.to_le_bytes());
    input.extend_from_slice(&khronos.to_be_bytes());

    let key = [
        0x4a,
        0x40,
        0x16,
        (nonce >> 8) as u8,
        0x47,
        0x6c,
        0x01,
        nonce as u8,
    ];
    let encrypted = transform(&input, &key);
    let mut result = Vec::with_capacity(6 + encrypted.len());
    result.extend_from_slice(&[0x84, 0x04, nonce as u8, (nonce >> 8) as u8, 0x40, 0x01]);
    result.extend_from_slice(&encrypted);
    result.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn transform(input: &[u8], key: &[u8; 8]) -> Vec<u8> {
    let mut table: [u8; 256] = std::array::from_fn(|index| index as u8);
    let mut cursor = 0_usize;
    for index in 0..256 {
        cursor = (cursor + usize::from(table[index]) + usize::from(key[index & 7])) & 255;
        table[index] = table[cursor];
    }
    let mut output = Vec::with_capacity(input.len());
    let mut position = 0_usize;
    cursor = 0;
    for byte in input {
        position += 1;
        let current = table[position];
        cursor += usize::from(current);
        let next = table[cursor & 255];
        table[position] = next;
        output.push(byte ^ table[(usize::from(next) * 2) & 255]);
    }
    for index in 0..output.len() {
        let rotated = output[index].rotate_left(4);
        let next = output[(index + 1) % output.len()];
        output[index] = !(reverse_bits(next ^ rotated) ^ 20);
    }
    output
}

fn reverse_bits(mut value: u8) -> u8 {
    let mut result = 0;
    for _ in 0..8 {
        result = (result << 1) | (value & 1);
        value >>= 1;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_and_post_match_public_reference_vectors() {
        assert_eq!(
            create(b"a=1&b=%2B", None, 1_700_000_000, 0x1234, 67_503_104),
            "8404341240014df6fc3ed9fe6cc91364e71df6105a4612699358"
        );
        assert_eq!(
            create(
                b"a=1&b=%2B",
                Some(&[0, 1, 2, 255]),
                1_700_000_000,
                0x1234,
                67_503_104,
            ),
            "8404341240014df6fc1eb323c4181364e71df6105a4612699358"
        );
    }
}
