//! Software ticket-guard primitives and client-data codec. Account binding,
//! mode selection, timestamp derivation and server acceptance remain separate.
use base64::{Engine, engine::general_purpose::STANDARD};
use ring::{
    rand::SystemRandom,
    signature::{self, EcdsaKeyPair, KeyPair},
};
use serde_json::Value;

/// Token-associated server proofs, kept separate from private signing keys.
/// Their presence does not establish the required mode or account binding.
pub struct TokenProofs {
    tee: Option<String>,
    ree: Option<String>,
}

impl TokenProofs {
    pub fn parse(encoded: &str) -> Result<Self, String> {
        if encoded.len() > 32768 {
            return Err("账号签名证明过大".into());
        }
        fn proof(value: Option<&Value>) -> Result<Option<String>, String> {
            match value {
                None | Some(Value::Null) => Ok(None),
                Some(Value::String(text)) => validate_proof(text),
                _ => Err("账号签名证明字段类型无效".into()),
            }
        }
        // The SDK first reads CombineTsSign JSON. A legacy opaque value is
        // assigned to ts_sign (TEE), never to ts_sign_ree.
        if let Ok(Value::Object(object)) = serde_json::from_str::<Value>(encoded) {
            Ok(Self {
                tee: proof(object.get("ts_sign"))?,
                ree: proof(object.get("ts_sign_ree"))?,
            })
        } else {
            Ok(Self {
                tee: validate_proof(encoded)?,
                ree: None,
            })
        }
    }

    pub fn tee(&self) -> Option<&str> {
        self.tee.as_deref()
    }

    pub fn ree(&self) -> Option<&str> {
        self.ree.as_deref()
    }

    /// Select only an explicitly supplied software proof. This is not a mode
    /// selector and must not be used to bypass a required hardware proof.
    pub fn require_ree(&self) -> Result<&str, String> {
        self.ree().ok_or_else(|| "账号缺少 REE 服务端证明".into())
    }
}

fn validate_proof(text: &str) -> Result<Option<String>, String> {
    if text.len() > 16384 || text.chars().any(char::is_control) || matches!(text, "NULL" | "null") {
        return Err("账号签名证明格式无效".into());
    }
    Ok((!text.is_empty()).then(|| text.to_owned()))
}

/// Software client-data fields only, not the complete HTTP guard header set.
/// Required TEE or attestation fields must be added by the owning protocol.
pub struct SoftwareClientData(String);

impl SoftwareClientData {
    pub fn header_value(&self) -> &str {
        &self.0
    }
}

fn software_client_data(
    timestamp: u64,
    ts_sign_ree: &str,
    signature: &[u8],
) -> Result<SoftwareClientData, String> {
    if ts_sign_ree.len() > 16384
        || ts_sign_ree.chars().any(char::is_control)
        || matches!(ts_sign_ree, "NULL" | "null")
    {
        return Err("软件签名的服务端证明格式无效".into());
    }
    // requestContentText is transient in the SDK. Never serialize the raw
    // message (which contains the ticket), key material or session snapshot.
    let value = serde_json::json!({"req_content":"ticket,path,timestamp", "timestamp":timestamp,
        "ts_sign":"", "ts_sign_ree":ts_sign_ree, "req_sign":"", "req_sign_ree":STANDARD.encode(signature)});
    let bytes = serde_json::to_vec(&value).map_err(|_| "客户端签名数据编码失败")?;
    Ok(SoftwareClientData(STANDARD.encode(bytes)))
}

pub struct HmacSigner {
    key: ring::hmac::Key,
}

impl HmacSigner {
    pub fn client_data(
        &self,
        ticket: &str,
        path: &str,
        timestamp: u64,
        ts_sign_ree: &str,
    ) -> Result<SoftwareClientData, String> {
        software_client_data(
            timestamp,
            ts_sign_ree,
            &self.sign_request(ticket, path, timestamp)?,
        )
    }

    pub fn from_snapshot(snapshot: &Value) -> Result<Self, String> {
        let encoded = snapshot["hmac_key"]
            .as_str()
            .ok_or("软件签名快照缺少 HMAC 密钥")?;
        if encoded.is_empty() || encoded.len() > 2048 || matches!(encoded, "NULL" | "null") {
            return Err("HMAC 密钥格式无效".into());
        }
        let bytes = STANDARD.decode(encoded).map_err(|_| "HMAC 密钥格式无效")?;
        if bytes.is_empty() || bytes.len() > 1024 {
            return Err("HMAC 密钥格式无效".into());
        }
        Ok(Self {
            key: ring::hmac::Key::new(ring::hmac::HMAC_SHA256, &bytes),
        })
    }

    pub fn sign(&self, message: &[u8]) -> Vec<u8> {
        ring::hmac::sign(&self.key, message).as_ref().to_vec()
    }

    pub fn verify(&self, message: &[u8], signature: &[u8]) -> bool {
        ring::hmac::verify(&self.key, message, signature).is_ok()
    }

    pub fn sign_request(
        &self,
        ticket: &str,
        path: &str,
        timestamp: u64,
    ) -> Result<Vec<u8>, String> {
        Ok(self.sign(request_message(ticket, path, timestamp)?.as_bytes()))
    }

    pub fn verify_request(
        &self,
        ticket: &str,
        path: &str,
        timestamp: u64,
        signature: &[u8],
    ) -> bool {
        request_message(ticket, path, timestamp)
            .is_ok_and(|message| self.verify(message.as_bytes(), signature))
    }
}

pub struct ReeSigner {
    key: EcdsaKeyPair,
}

impl ReeSigner {
    /// Compare a token's recorded software public key with this validated pair.
    /// Matching device keys alone does not establish the token's account.
    pub fn matches_public_key_base64(&self, encoded: &str) -> Result<bool, String> {
        if encoded.is_empty() || encoded.len() > 128 {
            return Err("Token 关联的软件公钥格式无效".into());
        }
        let point = STANDARD
            .decode(encoded)
            .map_err(|_| "Token 关联的软件公钥编码无效")?;
        if point.len() != 65 || point[0] != 4 {
            return Err("Token 关联的软件公钥格式无效".into());
        }
        Ok(point == self.key.public_key().as_ref())
    }

    pub fn client_data(
        &self,
        ticket: &str,
        path: &str,
        timestamp: u64,
        ts_sign_ree: &str,
    ) -> Result<SoftwareClientData, String> {
        software_client_data(
            timestamp,
            ts_sign_ree,
            &self.sign_request(ticket, path, timestamp)?,
        )
    }

    /// Import the exact scalar-hex / uncompressed-point-base64 formats emitted
    /// by the Android subprocess-sign provider, verifying they form one pair.
    /// This does not establish which account or device owns the key.
    pub fn from_snapshot(snapshot: &Value) -> Result<Self, String> {
        let private = snapshot["ree_private_key"]
            .as_str()
            .ok_or("软件签名快照缺少私钥")?;
        let public = snapshot["ree_public_key"]
            .as_str()
            .ok_or("软件签名快照缺少公钥")?;
        let mut scalar = [0u8; 32];
        if private.len() != 64 {
            return Err("软件签名私钥格式无效".into());
        }
        fn nibble(byte: u8) -> Option<u8> {
            match byte {
                b'0'..=b'9' => Some(byte - b'0'),
                b'a'..=b'f' => Some(byte - b'a' + 10),
                b'A'..=b'F' => Some(byte - b'A' + 10),
                _ => None,
            }
        }
        for (slot, pair) in scalar.iter_mut().zip(private.as_bytes().as_chunks::<2>().0) {
            *slot = (nibble(pair[0]).ok_or("软件签名私钥格式无效")? << 4)
                | nibble(pair[1]).ok_or("软件签名私钥格式无效")?;
        }
        if public.len() > 128 {
            return Err("软件签名公钥格式无效".into());
        }
        let point = STANDARD
            .decode(public)
            .map_err(|_| "软件签名公钥格式无效")?;
        if point.len() != 65 || point[0] != 4 {
            return Err("软件签名公钥格式无效".into());
        }
        let key = EcdsaKeyPair::from_private_key_and_public_key(
            &signature::ECDSA_P256_SHA256_ASN1_SIGNING,
            &scalar,
            &point,
            &SystemRandom::new(),
        )
        .map_err(|_| "软件签名密钥无效或公私钥不匹配")?;
        Ok(Self { key })
    }

    pub fn sign(&self, message: &[u8]) -> Result<Vec<u8>, String> {
        self.key
            .sign(&SystemRandom::new(), message)
            .map(|sig| sig.as_ref().to_vec())
            .map_err(|_| "软件签名失败".into())
    }

    pub fn verify(&self, message: &[u8], der_signature: &[u8]) -> bool {
        signature::UnparsedPublicKey::new(
            &signature::ECDSA_P256_SHA256_ASN1,
            self.key.public_key().as_ref(),
        )
        .verify(message, der_signature)
        .is_ok()
    }

    /// Sign the SDK's ticket/path/timestamp message. The caller must supply
    /// the correct SDK-selected ticket and path; account binding and mode
    /// selection are not established by this primitive.
    pub fn sign_request(
        &self,
        ticket: &str,
        path: &str,
        timestamp: u64,
    ) -> Result<Vec<u8>, String> {
        self.sign(request_message(ticket, path, timestamp)?.as_bytes())
    }

    pub fn verify_request(
        &self,
        ticket: &str,
        path: &str,
        timestamp: u64,
        signature: &[u8],
    ) -> bool {
        request_message(ticket, path, timestamp)
            .is_ok_and(|message| self.verify(message.as_bytes(), signature))
    }
}

fn request_message(ticket: &str, path: &str, timestamp: u64) -> Result<String, String> {
    if ticket.is_empty()
        || ticket.len() > 16384
        || ticket.chars().any(char::is_control)
        || !path.starts_with('/')
        || path.starts_with("//")
        || path.len() > 4096
        || path.contains(['?', '#', '\\'])
        || path.chars().any(char::is_control)
        || timestamp == 0
        || timestamp > i64::MAX as u64
    {
        return Err("签名请求的 ticket、路径或时间戳无效".into());
    }
    // X.1ZVK.LIZIZ/LIZJ concatenate these strings literally. URL encoding or
    // query serialization here would change the bytes signed by the app.
    Ok(format!("ticket={ticket}&path={path}&timestamp={timestamp}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn token_proofs_match_sdk_model_fixture_and_preserve_mode_separation() {
        let cases: Value = serde_json::from_str(include_str!(
            "../assets/guard-fixtures/token-proofs-40.8.json"
        ))
        .unwrap();
        for case in cases.as_array().unwrap() {
            let proofs = TokenProofs::parse(case["input"].as_str().unwrap()).unwrap();
            assert_eq!(proofs.tee(), case["ts_sign"].as_str());
            assert_eq!(proofs.ree(), case["ts_sign_ree"].as_str());
            assert_eq!(proofs.require_ree().is_ok(), proofs.ree().is_some());
        }
        let proofs = TokenProofs::parse(
            r#"{"ts_sign":"tee-only-proof","ts_sign_ree":"separate-ree-proof"}"#,
        )
        .unwrap();
        let header = ReeSigner::from_snapshot(&fixture())
            .unwrap()
            .client_data(
                "fixture-ticket",
                "/x/",
                1700000000,
                proofs.require_ree().unwrap(),
            )
            .unwrap();
        let payload: Value =
            serde_json::from_slice(&STANDARD.decode(header.header_value()).unwrap()).unwrap();
        assert_eq!(payload["ts_sign_ree"], "separate-ree-proof");
        assert!(!payload.to_string().contains("tee-only-proof"));
    }

    #[test]
    fn invalid_token_proofs_do_not_expose_values_or_select_software_fallback() {
        for input in [
            "NULL".to_owned(),
            "null".to_owned(),
            "private-secret\n".to_owned(),
            r#"{"ts_sign_ree":{"private-secret":1}}"#.to_owned(),
            r#"{"ts_sign_ree":"private-secret\n"}"#.to_owned(),
            "private-secret".repeat(3000),
        ] {
            let error = TokenProofs::parse(&input).err().unwrap();
            assert!(!error.contains("private-secret"));
        }
        for input in ["", "fixture-legacy", r#"{"ts_sign":"fixture-tee"}"#, "{}"] {
            assert!(TokenProofs::parse(input).unwrap().require_ree().is_err());
        }
    }

    #[test]
    fn software_envelope_contains_proofs_not_credentials_or_transient_text() {
        let signer = ReeSigner::from_snapshot(&fixture()).unwrap();
        let header = signer
            .client_data(
                "private-ticket",
                "/aweme/v1/feed/",
                1700000000,
                "fixture-proof",
            )
            .unwrap();
        assert!(!header.header_value().contains('\n'));
        let bytes = STANDARD.decode(header.header_value()).unwrap();
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["req_content"], "ticket,path,timestamp");
        assert_eq!(value["timestamp"], 1700000000u64);
        assert_eq!(value["ts_sign_ree"], "fixture-proof");
        assert_eq!(value["ts_sign"], "");
        assert_eq!(value["req_sign"], "");
        assert_eq!(value.as_object().unwrap().len(), 6);
        assert!(!String::from_utf8(bytes).unwrap().contains("private-ticket"));
        let signature = STANDARD
            .decode(value["req_sign_ree"].as_str().unwrap())
            .unwrap();
        assert!(signer.verify_request("private-ticket", "/aweme/v1/feed/", 1700000000, &signature));
        assert!(
            signer
                .client_data("private-ticket", "/x/", 1700000000, "NULL")
                .is_err()
        );
    }

    #[test]
    fn hmac_matches_rfc4231_and_rejects_invalid_snapshots() {
        let signer =
            HmacSigner::from_snapshot(&json!({"hmac_key":STANDARD.encode([0x0b;20])})).unwrap();
        let signature = signer.sign(b"Hi There");
        let hex: String = signature.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(
            hex,
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
        assert!(signer.verify(b"Hi There", &signature));
        assert!(!signer.verify(b"changed", &signature));
        assert!(!signer.verify(b"Hi There", &[0; 32]));
        let request = signer
            .sign_request("fixture", "/aweme/v1/feed/", 1700000000)
            .unwrap();
        assert!(signer.verify_request("fixture", "/aweme/v1/feed/", 1700000000, &request));
        assert!(!signer.verify_request("other", "/aweme/v1/feed/", 1700000000, &request));
        for encoded in ["", "NULL", "null", "private-secret!"] {
            let error = HmacSigner::from_snapshot(&json!({"hmac_key":encoded}))
                .err()
                .unwrap();
            assert!(!error.contains("private-secret"));
        }
    }

    #[test]
    fn software_envelope_matches_real_sdk_serializer_fixture() {
        let header = software_client_data(1700000000, "fixture-proof", &[1, 2, 3]).unwrap();
        let actual: Value =
            serde_json::from_slice(&STANDARD.decode(header.header_value()).unwrap()).unwrap();
        let expected: Value = serde_json::from_str(include_str!(
            "../assets/guard-fixtures/client-data-40.8.json"
        ))
        .unwrap();
        assert_eq!(actual, expected);
    }

    fn fixture() -> Value {
        // Public P-256 generator test vector, scalar = 1. Never a real key.
        let hex = "046b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c2964fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5";
        let point: Vec<u8> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect();
        json!({"ree_private_key":format!("{:064x}",1),"ree_public_key":STANDARD.encode(point)})
    }

    #[test]
    fn signs_der_and_rejects_modified_messages_and_mismatched_pairs() {
        let mut snapshot = fixture();
        let signer = ReeSigner::from_snapshot(&snapshot).unwrap();
        let sig = signer.sign(b"public-test-message").unwrap();
        assert!(signer.verify(b"public-test-message", &sig));
        assert!(!signer.verify(b"changed-message", &sig));
        assert!(!signer.verify(b"public-test-message", &[0; 64]));
        snapshot["ree_private_key"] = json!(format!("{:064x}", 2));
        assert!(ReeSigner::from_snapshot(&snapshot).is_err());
    }

    #[test]
    fn token_public_key_comparison_rejects_changed_or_malformed_metadata() {
        let snapshot = fixture();
        let signer = ReeSigner::from_snapshot(&snapshot).unwrap();
        let public = snapshot["ree_public_key"].as_str().unwrap();
        assert!(signer.matches_public_key_base64(public).unwrap());
        let mut changed = STANDARD.decode(public).unwrap();
        changed[1] ^= 1;
        assert!(
            !signer
                .matches_public_key_base64(&STANDARD.encode(changed))
                .unwrap()
        );
        for invalid in ["", "NULL", "private-secret!", "YWJjZA=="] {
            let error = signer.matches_public_key_base64(invalid).unwrap_err();
            assert!(!error.contains("private-secret"));
        }
    }

    #[test]
    fn rejects_null_and_malformed_credentials_without_echoing_them() {
        for input in [
            json!({}),
            json!({"ree_private_key":"private-secret","ree_public_key":"NULL"}),
            json!({"ree_private_key":"0".repeat(64),"ree_public_key":"private-secret"}),
        ] {
            let error = ReeSigner::from_snapshot(&input).err().unwrap();
            assert!(!error.contains("private-secret"));
        }
    }

    #[test]
    fn sdk_request_bytes_are_literal_and_signatures_bind_every_component() {
        let ticket = "fixture%2F+opaque=tail";
        let path = "/aweme/v1/%E6%B5%8B/";
        assert_eq!(
            request_message(ticket, path, 1700000000).unwrap(),
            "ticket=fixture%2F+opaque=tail&path=/aweme/v1/%E6%B5%8B/&timestamp=1700000000"
        );
        let signer = ReeSigner::from_snapshot(&fixture()).unwrap();
        let signature = signer.sign_request(ticket, path, 1700000000).unwrap();
        assert!(signer.verify_request(ticket, path, 1700000000, &signature));
        assert!(!signer.verify_request("changed", path, 1700000000, &signature));
        assert!(!signer.verify_request(ticket, "/changed/", 1700000000, &signature));
        assert!(!signer.verify_request(ticket, path, 1700000001, &signature));
        for invalid in [
            "https://outside.invalid/",
            "//outside.invalid/",
            "/x?secret",
            "/x\n",
        ] {
            let error = request_message("private-ticket", invalid, 1700000000).unwrap_err();
            assert!(!error.contains("private-ticket"));
            assert!(!error.contains("secret"));
        }
    }
}
