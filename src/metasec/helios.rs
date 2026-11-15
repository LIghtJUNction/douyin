//! Helios value construction from the recovered Metasec module at `0x43eb90`.
//! Callers must supply the genuine SDK context and secret; this is not a signer.

use super::speck::SpeckKey;
use base64::{Engine, engine::general_purpose::STANDARD};
use md5::{Digest, Md5};
use std::{error::Error, fmt};

/// Invalid C-string input supplied to the Helios value constructor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EmbeddedNul;

impl fmt::Display for EmbeddedNul {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SDK context text and aid must not contain NUL")
    }
}

impl Error for EmbeddedNul {}

/// Construct a Helios value using an explicit nonce and SDK-provided inputs.
pub fn create_value(
    timestamp_seconds: u32,
    context_text: &str,
    aid: &str,
    secret: &[u8],
    nonce: [u8; 4],
) -> Result<String, EmbeddedNul> {
    if context_text.contains('\0') || aid.contains('\0') {
        return Err(EmbeddedNul);
    }
    let mut hasher = Md5::new();
    hasher.update(nonce);
    hasher.update(secret);
    let digest = hasher.finalize();
    let mut key = [0; 32];
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for (index, byte) in digest.iter().enumerate() {
        key[index * 2] = HEX[usize::from(byte >> 4)];
        key[index * 2 + 1] = HEX[usize::from(byte & 15)];
    }
    let cipher = SpeckKey::new(&key);
    let mut plaintext = format!("{timestamp_seconds}-{context_text}-{aid}").into_bytes();
    let ciphertext = cipher.encrypt_padded(&plaintext);
    plaintext.fill(0);
    key.fill(0);
    drop(cipher);
    let mut result = Vec::with_capacity(4 + ciphertext.len());
    result.extend_from_slice(&nonce);
    result.extend_from_slice(&ciphertext);
    Ok(STANDARD.encode(result))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_value_matches_the_public_reference_vector() {
        let value = create_value(
            1700000000,
            "synthetic-context",
            "1128",
            &[0xff, 0, 0xc3, 0x28],
            [1, 0x23, 0x45, 0x67],
        )
        .unwrap();
        assert_eq!(
            value,
            "ASNFZ4R9IWE5RquXjXa2zBsu0D2K1yWi2tPHmkbt3O9Rskf1mRCI+iCemjrsHJCNFCfeZg=="
        );
        let decoded = STANDARD.decode(value).unwrap();
        assert_eq!(decoded[..4], [1, 0x23, 0x45, 0x67]);
        assert!(create_value(0, "bad\0context", "1128", &[], [0; 4]).is_err());
        assert!(create_value(0, "context", "bad\0aid", &[], [0; 4]).is_err());
    }
}
