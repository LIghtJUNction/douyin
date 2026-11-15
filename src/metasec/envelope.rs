//! Medusa envelope transforms independent of application-specific constants.
//! These functions do not construct the message or establish an SDK session.

use sm3::Sm3;
use std::{error::Error, fmt};

/// Input rejected by the recovered `xmxor` transform.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputTooShort;

impl fmt::Display for InputTooShort {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("xmxor input must contain at least four bytes")
    }
}

impl Error for InputTooShort {}

/// Apply the recovered byte transform using a 32-byte SM3 key.
pub fn xmxor(data: &[u8], key: &[u8; 32]) -> Result<Vec<u8>, InputTooShort> {
    if data.len() < 4 {
        return Err(InputTooShort);
    }
    let mut encoded = vec![0; data.len()];
    for (index, byte) in data.iter().enumerate() {
        let offset = (index * 4) & 28;
        let first = key[offset];
        let second = key[offset + 1];
        let mixed = !(u32::from(byte.rotate_left(4)) + u32::from(first)) ^ u32::from(second);
        let rotated = (mixed as u8).rotate_left(3);
        let value = rotated.wrapping_add(second) ^ first;
        encoded[data.len() - index - 1] = !value;
    }
    let last = encoded[data.len() - 1] ^ encoded[data.len() - 2];
    let first = encoded[0];
    encoded[0] = (!last).wrapping_add(first);
    encoded[1] = (encoded[0] ^ encoded[data.len() - 1] ^ 254).wrapping_add(encoded[1]);
    encoded[2] =
        encoded[2].wrapping_add(((last.wrapping_sub(first)) ^ encoded[1].rotate_left(3)) ^ 2);
    for index in 0..data.len() - 4 {
        let mixed = encoded[index + 2].rotate_left(3) ^ encoded[index + 1] ^ (index + 3) as u8;
        encoded[index + 3] = (!mixed).wrapping_add(encoded[index + 3]);
    }
    encoded[data.len() - 1] ^= encoded[data.len() - 2];
    let sum = encoded[1..]
        .iter()
        .fold(0_u32, |sum, byte| sum.wrapping_add(u32::from(*byte)));
    encoded[0] = (encoded[0] ^ encoded[1]).wrapping_add(sum as u8);
    Ok(encoded)
}

/// Derive the envelope SM3 key and four-byte mask from a request nonce.
#[must_use]
pub fn key_hash(sign_key: &[u8; 32], nonce: u32) -> ([u8; 32], [u8; 4]) {
    let mut hasher = <Sm3 as Default>::default();
    sm3::digest::Digest::update(&mut hasher, sign_key);
    sm3::digest::Digest::update(&mut hasher, nonce.to_le_bytes());
    sm3::digest::Digest::update(&mut hasher, sign_key);
    let hash = sm3::digest::Digest::finalize(hasher).into();
    let middle = (nonce >> 16) & 255;
    let seed = !(((middle << 11) | (nonce >> 24)) ^ (middle >> 5) ^ middle);
    (hash, seed.to_le_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transforms_match_public_reference_vectors() {
        let data: [u8; 32] = std::array::from_fn(|index| index as u8);
        let key: [u8; 32] = std::array::from_fn(|index| index as u8 + 32);
        assert_eq!(
            hex(&xmxor(&data, &key).unwrap()),
            "e4b736dfa95e1d11ddfd2b31cb8fc5501f2ee3caac1320c55cad4034743fb374"
        );
        let sign_key: [u8; 32] = std::array::from_fn(|index| index as u8);
        let (hash, seed) = key_hash(&sign_key, 0x1234_5678);
        assert_eq!(
            hex(&hash),
            "88b8676e700a0937dfe804ab8f7aa7b482f0d2a147ea1d80b20db8320423649c"
        );
        assert_eq!(hex(&seed), "d85ffeff");
        assert_eq!(xmxor(&[1, 2, 3], &key), Err(InputTooShort));
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}
