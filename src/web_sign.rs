//! Douyin web `a_bogus` generation with an explicit page/application identity.
//! Query, body and User-Agent must exactly match the eventual request.

use md5::{Digest, Md5};
use ring::rand::SecureRandom;
use std::time::{SystemTime, UNIX_EPOCH};

const SALT: &str = "dhzx";
const HEADER_MAGIC: [u8; 2] = [3, 82];
const SDK_VERSION: [u8; 4] = [1, 0, 1, 0];
const PAYLOAD_KEY: u8 = 0xD3;
const FORTNIGHT_EPOCH_MS: u64 = 1_721_836_800_000;
const FORTNIGHT_MS: u64 = 1000 * 60 * 60 * 24 * 14;
const FIELD_ORDER: [usize; 50] = [
    34, 44, 56, 61, 73, 29, 70, 45, 35, 49, 38, 66, 51, 68, 28, 48, 64, 47, 30, 71, 26, 55, 31, 69,
    59, 40, 62, 63, 27, 72, 41, 74, 57, 52, 42, 39, 33, 67, 53, 43, 65, 46, 36, 24, 60, 32, 79, 80,
    84, 85,
];
const ENV_FLAGS: u16 = 1;
const DETECT_FLAGS: u32 = 14;
const NR_FLAGS: u16 = 0x21;
const NR_TAG: [u8; 4] = [0, 0, 0, 0];
const TRIPWIRE_LOCKED: u8 = 3;
const CALL_BUCKET: u8 = 6;
const DEFAULT_BROWSER_INFO: &str = "1536|742|1536|864|1536|864|1536|864|MacIntel";
const CANARIES: [(usize, u8, u8); 3] = [(3, 11, 12), (4, 8, 9), (5, 12, 13)];
const TRIPWIRE_SET: u8 = 0xB2;
const TRIPWIRE_FREE: u8 = 0x4D;
const S3: &[u8; 64] = b"ckdp1h4ZKsUB80/Mfvw36XIgR25+WQAlEi7NLboqYTOPuzmFjJnryx9HVGDaStCe";
const S4: &[u8; 64] = b"Dkdpgh2ZmsQB80/MfvV36XI1R45-WUAlEixNLwoqYTOPuzKFjJnry79HbGcaStCe";
const NOISE_MASKS: [u8; 3] = [0x91, 0x42, 0x2C];
const DATA_MASKS: [u8; 3] = [0x6E, 0xBD, 0xD3];

const TOKEN_ALPHANUMERIC: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
const VERIFY_FP_CHARS: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

/// Generate a browser-style token using the OS random source.
pub fn generate_ms_token(length: usize) -> String {
    if length == 0 {
        return String::new();
    }
    let mut entropy = vec![0; length];
    ring::rand::SystemRandom::new()
        .fill(&mut entropy)
        .expect("the operating system random source must be available");
    entropy
        .iter()
        .map(|byte| TOKEN_ALPHANUMERIC[usize::from(*byte) % TOKEN_ALPHANUMERIC.len()] as char)
        .collect()
}

/// Generate the browser verifyFp value for a millisecond timestamp.
pub fn generate_verify_fp(timestamp_ms: u64) -> String {
    let mut entropy = [0; 36];
    ring::rand::SystemRandom::new()
        .fill(&mut entropy)
        .expect("the operating system random source must be available");
    let mut chars = ['\0'; 36];
    chars[8] = '_';
    chars[13] = '_';
    chars[18] = '_';
    chars[23] = '_';
    chars[14] = '4';
    let mut index = 0;
    for (position, slot) in chars.iter_mut().enumerate() {
        if *slot != '\0' {
            continue;
        }
        let random = usize::from(entropy[index % entropy.len()]) % VERIFY_FP_CHARS.len();
        index += 1;
        *slot = VERIFY_FP_CHARS[if position == 19 {
            (random & 3) | 8
        } else {
            random
        }] as char;
    }
    format!(
        "verify_{}_{}",
        base36(timestamp_ms),
        chars.iter().collect::<String>()
    )
}

fn base36(mut value: u64) -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if value == 0 {
        return "0".into();
    }
    let mut output = Vec::new();
    while value > 0 {
        output.push(DIGITS[(value % 36) as usize] as char);
        value /= 36;
    }
    output.reverse();
    output.into_iter().collect()
}

const WEB_SIGN_SALT: &str = "A96D855A08C0A9707F8BEF0D9A527E4E";

fn decoded_query_pairs(query: &str) -> Result<Vec<(String, String)>, String> {
    query
        .split('&')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let (name, value) = part.split_once('=').unwrap_or((part, ""));
            let decode = |input: &str| -> Result<String, String> {
                percent_encoding::percent_decode_str(input)
                    .decode_utf8()
                    .map(|value| value.into_owned())
                    .map_err(|_| "网页查询参数不是有效 UTF-8".into())
            };
            Ok((decode(name)?, decode(value)?))
        })
        .collect()
}

fn encode_query_pairs(pairs: &[(String, String)]) -> String {
    pairs
        .iter()
        .map(|(name, value)| {
            format!(
                "{}={}",
                url::form_urlencoded::byte_serialize(name.as_bytes()).collect::<String>(),
                url::form_urlencoded::byte_serialize(value.as_bytes()).collect::<String>()
            )
        })
        .collect::<Vec<_>>()
        .join("&")
}

/// Canonicalize a raw query with the WebSign URLSearchParams encoding rules.
pub fn canonical_query(query: &str) -> Result<String, String> {
    Ok(encode_query_pairs(&decoded_query_pairs(query)?))
}

/// Append the visitor timestamp and compute the Douyin WebSign query signature.
pub fn web_signature(query: &str, uifid: &str, timestamp: u64) -> Result<(String, String), String> {
    if uifid.is_empty() {
        return Err("网页访客标识不能为空".into());
    }
    let mut pairs = decoded_query_pairs(query)?;
    if pairs
        .iter()
        .any(|(name, value)| name == "uifid" && value != uifid)
    {
        return Err("网页查询参数与 Cookie 的访客标识不一致".into());
    }
    if pairs.iter().any(|(name, _)| name == "timestamp") {
        return Err("网页查询参数不能覆盖 WebSign 时间戳".into());
    }
    if !pairs.iter().any(|(name, _)| name == "uifid") {
        pairs.push(("uifid".into(), uifid.into()));
    }
    pairs.push(("timestamp".into(), timestamp.to_string()));
    let covered = encode_query_pairs(&pairs);
    let signature = format!(
        "{:x}",
        Md5::digest(format!("{uifid}_{timestamp}_{WEB_SIGN_SALT}_{covered}").as_bytes())
    );
    Ok((
        format!("{covered}&x-secsdk-web-signature={signature}"),
        signature,
    ))
}

pub struct WebSigner {
    pub page_id: u32,
    pub aid: u32,
}

impl Default for WebSigner {
    fn default() -> Self {
        Self {
            page_id: 6241,
            aid: 6383,
        }
    }
}

impl WebSigner {
    /// Sign the exact query/body strings for the current millisecond clock.
    pub fn sign(&self, query: &str, body: &str, user_agent: &str, now_ms: u64) -> String {
        let mut rng = XorShift::from_clock();
        self.sign_with_rng(query, body, user_agent, now_ms, &mut || rng.random())
    }

    fn sign_with_rng(
        &self,
        query: &str,
        body: &str,
        user_agent: &str,
        now_ms: u64,
        rng: &mut dyn FnMut() -> f64,
    ) -> String {
        let now_ms = now_ms.max(FORTNIGHT_EPOCH_MS);
        let fields = self.fields(query, body, user_agent, now_ms);
        let mut version = [0; 8];
        version[..4].copy_from_slice(&mask_pair(
            [SDK_VERSION[0], SDK_VERSION[1]],
            rng,
            None,
            None,
        ));
        let probe = probe_noise(rng);
        let tripwire = tripwire_noise(rng);
        version[4..].copy_from_slice(&mask_pair(
            [SDK_VERSION[2], SDK_VERSION[3]],
            rng,
            Some(probe),
            Some(tripwire),
        ));
        let mut checksum = version.iter().fold(0, |acc, byte| acc ^ byte);
        let mut body_bytes: Vec<u8> = FIELD_ORDER.iter().map(|index| fields[*index]).collect();
        for byte in &body_bytes {
            checksum ^= byte;
        }
        body_bytes.extend(js_bytes(DEFAULT_BROWSER_INFO));
        body_bytes.extend(js_bytes(&tail_text(now_ms)));
        body_bytes.push(checksum);
        let header_high = header_noise(user_agent, rng);
        let header = mask_pair(HEADER_MAGIC, rng, None, Some(header_high));
        let frame = expand_noise(&body_bytes, rng);
        let mut plain = version.to_vec();
        plain.extend(frame);
        let sealed = rc4(&[PAYLOAD_KEY], &plain);
        let mut payload = header.to_vec();
        payload.extend(sealed);
        encode_base64(&payload, S4)
    }

    fn fields(&self, query: &str, body: &str, user_agent: &str, now_ms: u64) -> [u8; 86] {
        let mut fields = [0; 86];
        fields[24] = 41;
        fields[26] = ((now_ms - FORTNIGHT_EPOCH_MS) / FORTNIGHT_MS) as u8;
        fields[27] = CALL_BUCKET;
        fields[28] = 3;
        fields[35] = ENV_FLAGS as u8;
        fields[36] = (ENV_FLAGS >> 8) as u8;
        fields[38] = NR_FLAGS as u8;
        fields[39] = (NR_FLAGS >> 8) as u8;
        fields[66] = TRIPWIRE_LOCKED;
        let info_len = js_bytes(DEFAULT_BROWSER_INFO).len();
        fields[79] = info_len as u8;
        fields[80] = (info_len >> 8) as u8;
        let tail_len = js_bytes(&tail_text(now_ms)).len();
        fields[84] = tail_len as u8;
        fields[85] = (tail_len >> 8) as u8;
        for (index, byte) in le_bytes(now_ms, 6).into_iter().enumerate() {
            fields[29 + index] = byte;
        }
        for (index, byte) in le_bytes(DETECT_FLAGS as u64, 4).into_iter().enumerate() {
            fields[44 + index] = byte;
        }
        for (index, byte) in NR_TAG.into_iter().enumerate() {
            fields[40 + index] = byte;
        }
        for (index, byte) in le_bytes(now_ms - 1, 6).into_iter().enumerate() {
            fields[60 + index] = byte;
        }
        for (index, byte) in le_bytes(self.page_id as u64, 4).into_iter().enumerate() {
            fields[67 + index] = byte;
        }
        for (index, byte) in le_bytes(self.aid as u64, 4).into_iter().enumerate() {
            fields[71 + index] = byte;
        }
        let chains = [
            ([48, 49, 51], [9, 18], CANARIES[0], digest_of(query)),
            ([52, 53, 55], [10, 19], CANARIES[1], digest_of(body)),
            (
                [56, 57, 59],
                [11, 21],
                CANARIES[2],
                user_agent_digest(user_agent),
            ),
        ];
        for (slots, indices, canary, digest) in chains {
            fields[slots[0]] = digest[indices[0]];
            fields[slots[1]] = digest[indices[1]];
            fields[slots[2]] = canary_value(&digest, canary.0, canary.1, canary.2);
        }
        fields
    }
}

fn tail_text(now_ms: u64) -> String {
    format!("{},", (now_ms + 3) & 0xFF)
}

fn le_bytes(value: u64, count: usize) -> Vec<u8> {
    (0..count)
        .map(|index| (value >> (8 * index)) as u8)
        .collect()
}

fn canary_value(digest: &[u8; 32], offset: usize, sentinel: u8, fallback: u8) -> u8 {
    digest[offset..]
        .iter()
        .copied()
        .find(|byte| *byte != sentinel)
        .unwrap_or(fallback)
}

fn sm3(data: &[u8]) -> [u8; 32] {
    let mut hasher = <sm3::Sm3 as Default>::default();
    sm3::digest::Digest::update(&mut hasher, data);
    sm3::digest::Digest::finalize(hasher).into()
}

fn digest_of(text: &str) -> [u8; 32] {
    let mut salted = text.as_bytes().to_vec();
    salted.extend_from_slice(SALT.as_bytes());
    sm3(&sm3(&salted))
}

fn user_agent_digest(user_agent: &str) -> [u8; 32] {
    let key = [(ENV_FLAGS >> 8) as u8, ENV_FLAGS as u8, DETECT_FLAGS as u8];
    let sealed = rc4(&key, &js_bytes(user_agent.trim()));
    sm3(encode_base64(&sealed, S3).as_bytes())
}

fn rc4(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut sbox = [0; 256];
    for (index, slot) in sbox.iter_mut().rev().enumerate() {
        *slot = index as u8;
    }
    let mut j = 0;
    for index in 0..256 {
        j = (j * sbox[index] as usize + j + key[index % key.len()] as usize) % 256;
        sbox.swap(index, j);
    }
    let (mut i, mut j) = (0, 0);
    data.iter()
        .map(|byte| {
            i = (i + 1) % 256;
            j = (j + sbox[i] as usize) % 256;
            sbox.swap(i, j);
            byte ^ sbox[(sbox[i] as usize + sbox[j] as usize) % 256]
        })
        .collect()
}

fn encode_base64(data: &[u8], table: &[u8]) -> String {
    let mut output = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let mut block = 0_u32;
        for (index, byte) in chunk.iter().enumerate() {
            block |= u32::from(*byte) << (16 - 8 * index);
        }
        for shift in [18, 12, 6, 0].iter().take(chunk.len() + 1) {
            output.push(table[((block >> shift) & 63) as usize] as char);
        }
    }
    while !output.len().is_multiple_of(4) {
        output.push('=');
    }
    output
}

fn js_bytes(text: &str) -> Vec<u8> {
    let mut output = Vec::with_capacity(text.len());
    for code in text.encode_utf16() {
        if code & 0xFF00 != 0 {
            output.push((code >> 8) as u8);
        }
        output.push(code as u8);
    }
    output
}

fn mask_pair(
    pair: [u8; 2],
    rng: &mut dyn FnMut() -> f64,
    low: Option<u8>,
    high: Option<u8>,
) -> [u8; 4] {
    let noise = (rng() * 65535.0) as u32;
    let low = low.unwrap_or((noise & 255) as u8);
    let high = high.unwrap_or(((noise >> 8) & 255) as u8);
    [
        (low & 0xAA) | (pair[0] & 0x55),
        (low & 0x55) | (pair[0] & 0xAA),
        (high & 0xAA) | (pair[1] & 0x55),
        (high & 0x55) | (pair[1] & 0xAA),
    ]
}

fn expand_noise(body: &[u8], rng: &mut dyn FnMut() -> f64) -> Vec<u8> {
    let mut output = Vec::with_capacity(body.len() / 3 * 4 + 2);
    for group in body.chunks(3) {
        if group.len() < 3 {
            output.push(group[0]);
            if group.len() > 1 && group[1] != 0 {
                output.push(group[1]);
            }
            continue;
        }
        let noise = ((rng() * 1000.0) as u32 & 255) as u8;
        for index in 0..3 {
            output.push((noise & NOISE_MASKS[index]) | (group[index] & DATA_MASKS[index]));
        }
        output.push(
            (group[0] & NOISE_MASKS[0]) | (group[1] & NOISE_MASKS[1]) | (group[2] & NOISE_MASKS[2]),
        );
    }
    output
}

fn header_noise(user_agent: &str, rng: &mut dyn FnMut() -> f64) -> u8 {
    let ua = user_agent.to_ascii_lowercase();
    let base = if ua.contains("edg") {
        125
    } else if ua.contains("huawei") {
        170
    } else if ua.contains("firefox") {
        40
    } else if ua.contains("chrome") {
        0
    } else if ua.contains("safari") {
        81
    } else {
        210
    };
    ((base + (rng() * 40.0) as u32) & 255) as u8
}

fn probe_noise(rng: &mut dyn FnMut() -> f64) -> u8 {
    let value = (rng() * 240.0) as u32;
    let value = if value > 109 {
        value + value % 2 + 1
    } else {
        value
    };
    value as u8
}

fn tripwire_noise(rng: &mut dyn FnMut() -> f64) -> u8 {
    (((rng() * 255.0) as u32 as u8) & TRIPWIRE_FREE) | TRIPWIRE_SET
}

struct XorShift(u64);

impl XorShift {
    fn from_clock() -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos() as u64)
            .unwrap_or(0x9E37_79B9_7F4A_7C15);
        Self(nanos ^ 0x9E37_79B9_7F4A_7C15 | 1)
    }

    fn random(&mut self) -> f64 {
        let mut value = self.0;
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        self.0 = value;
        (value >> 11) as f64 / (1_u64 << 53) as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/153.0.0.0 Safari/537.36";
    const QUERY: &str = "filter=127%2C127%2C127%2C127&page_count=10&page_index=0&query_type=0&query_word=%E5%8D%81%E6%97%A5%E7%BB%88%E7%84%89";

    fn fixed_rng() -> impl FnMut() -> f64 {
        let mut index = 0_u64;
        move || {
            let value = ((index * 37) % 97) as f64 / 97.0;
            index += 1;
            value
        }
    }

    #[test]
    fn main_site_signature_matches_python_reference() {
        let mut rng = fixed_rng();
        let signature =
            WebSigner::default().sign_with_rng(QUERY, "", UA, 1_789_000_000_123, &mut rng);
        assert_eq!(
            signature,
            "QJ4fhHyEm2WnCdFtmOJUeHdlq06/NBuyYzTxbN9PCxTELwFTJ8Nr/Oc6rowLsPxRRYBTh917sxUlbDVcp4UhpCnkKmkDuxkRCt5A9hmohqqgGes/LND0e0gFKwseUQJNeA91EIhRXsMN2jnR9r5MlpMay5FH5REgSNBCpMSy7EW6DSukkn3kOCkkiLNzUUIb"
        );
    }

    #[test]
    fn web_sign_matches_the_public_reference_vector() {
        let (query, signature) = web_signature(
            "aid=6383&uifid=test_uifid&a_bogus=sig%2Bwith%2Fchars",
            "test_uifid",
            1_788_848_901,
        )
        .unwrap();
        assert_eq!(signature, "06ce4e840d33306b0a4367ca0c7e718d");
        assert_eq!(
            query,
            "aid=6383&uifid=test_uifid&a_bogus=sig%2Bwith%2Fchars&timestamp=1788848901&x-secsdk-web-signature=06ce4e840d33306b0a4367ca0c7e718d"
        );
        assert_eq!(
            canonical_query("keyword=hello%20world&literal=visitor+id").unwrap(),
            "keyword=hello+world&literal=visitor%2Bid"
        );
        assert!(web_signature("uifid=other", "test_uifid", 1).is_err());
        assert!(web_signature("timestamp=1", "test_uifid", 1).is_err());
    }

    #[test]
    fn browser_tokens_have_the_expected_shape() {
        assert_eq!(generate_ms_token(107).len(), 107);
        assert_eq!(base36(0), "0");
        assert_eq!(base36(35), "z");
        assert_eq!(base36(36), "10");
        let verify = generate_verify_fp(1_700_000_000_000);
        assert!(verify.starts_with("verify_loyw3v28_"));
        assert_eq!(verify.len(), "verify_loyw3v28_".len() + 36);
        assert_eq!(generate_ms_token(0), "");
    }

    #[test]
    fn primitives_match_standard_and_reference_vectors() {
        assert_eq!(
            hex(&sm3(b"abc")),
            "66c7f0f462eeedd9d1f2d46bdc10e4e24167c4875cf2f7a2297da02b8f4ba8e0"
        );
        assert_eq!(encode_base64(b"abcde", S3), "RIsN246=");
        assert_eq!(js_bytes("aé"), [0x61, 0xe9]);
        assert_eq!(js_bytes("中"), [0x4e, 0x2d]);
        assert_eq!(
            hex(&rc4(&[0xd3], &(0..16).collect::<Vec<u8>>())),
            "7c2122643dac8faa2aa974dbce90cd72"
        );
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}
