//! Public APK constant decoding, reconstructed from the pinned 40.8.0 fixtures.
//! This does not initialize Metasec or sign HTTP requests.

pub mod digest;
pub mod helios;
pub mod speck;

use serde_json::Value;

/// Configuration layout of the pinned APK's `ms.bd.c.u2` Java model.
///
/// This is a serializer, not an initialized/validated SDK configuration. In
/// particular its constructor defaults do not satisfy the app's init gates.
/// Map entries must already be in the desired Java iteration order; this does
/// not reproduce HashMap bucket order from insertion order.
#[derive(Clone)]
pub struct Configuration {
    pub app_id: Option<String>,
    pub alternate_app_id: Option<String>,
    pub license: Option<String>,
    pub sdk_version: Option<String>,
    pub device_id: Option<String>,
    pub install_id: Option<String>,
    /// Unmapped Java fields LJII, LIZIZ, LIZLLL, LJFF, in that order.
    pub opaque_text: [Option<String>; 4],
    /// Java integer fields LJIIJ and LJIIJJI.
    pub controls: [i32; 2],
    pub mode: i32,
    pub fallback_device_id: Option<String>,
    pub fallback_device_id_flag: i32,
    pub parameters: Vec<(Option<String>, Value)>,
    pub options: Vec<(Option<String>, Value)>,
}

impl Default for Configuration {
    fn default() -> Self {
        Self {
            app_id: None,
            alternate_app_id: None,
            license: None,
            sdk_version: None,
            device_id: None,
            install_id: None,
            opaque_text: Default::default(),
            controls: [-1, -1],
            mode: 99999,
            fallback_device_id: None,
            fallback_device_id_flag: 0,
            parameters: Vec::new(),
            options: Vec::new(),
        }
    }
}

fn java_trim(text: &str) -> &str {
    text.trim_matches(|c: char| c <= '\u{20}')
}

fn string_value(text: Option<&str>) -> Value {
    Value::String(java_trim(text.unwrap_or_default()).to_owned())
}

fn map_array(entries: &[(Option<String>, Value)]) -> Value {
    let mut result = Vec::new();
    for (key, value) in entries {
        // Java checks TextUtils.isEmpty before trim, not after it.
        if let Some(key) = key.as_deref().filter(|key| !key.is_empty()) {
            result.push(string_value(Some(key)));
            result.push(string_value(value.as_str()));
        }
    }
    Value::Array(result)
}

impl Configuration {
    pub fn to_wire_value(&self) -> Value {
        let mut result = [
            self.app_id.as_deref(),
            self.opaque_text[0].as_deref(),
            self.alternate_app_id.as_deref(),
            self.license.as_deref(),
            self.sdk_version.as_deref(),
            self.opaque_text[1].as_deref(),
            self.device_id.as_deref(),
            self.opaque_text[2].as_deref(),
            self.install_id.as_deref(),
            self.opaque_text[3].as_deref(),
        ]
        .into_iter()
        .map(string_value)
        .collect::<Vec<_>>();
        for value in [self.controls[0], self.controls[1], self.mode] {
            result.push(Value::String(value.to_string()));
        }
        result.push(string_value(self.fallback_device_id.as_deref()));
        result.push(Value::String(self.fallback_device_id_flag.to_string()));
        result.push(map_array(&self.parameters));
        result.push(map_array(&self.options));
        Value::Array(result)
    }

    /// Android JSONStringer also escapes forward slashes. Do not use generic
    /// Unicode whitespace trimming or sort map entries when encoding this model.
    pub fn to_wire_json(&self) -> String {
        self.to_wire_value().to_string().replace('/', "\\/")
    }
}

/// Decode the constant payload transform observed with command 0x1000001 and
/// zero integer/long arguments. The mask is public data from the APK library,
/// not an account key. JNI return semantics and other commands are separate.
pub fn decode_constant(seed: &str, payload: &[u8]) -> Result<String, &'static str> {
    const MASK: [u8; 10] = [0x71, 0x62, 0x13, 0x14, 0x5f, 0x77, 0x63, 0x41, 0x31, 0x30];
    let seed = seed.as_bytes();
    if seed.is_empty() {
        return Err("SDK constant seed must not be empty");
    }
    let decoded = payload
        .iter()
        .enumerate()
        .map(|(index, byte)| {
            let slot = index % MASK.len();
            byte ^ MASK[slot] ^ seed[slot % seed.len()]
        })
        .collect();
    String::from_utf8(decoded).map_err(|_| "SDK constant is not valid UTF-8")
}

#[cfg(test)]
mod tests {
    use super::{Configuration, decode_constant};
    use serde_json::Value;

    fn fixtures() -> Value {
        serde_json::from_str(include_str!("../assets/metasec-fixtures/config-wire.json")).unwrap()
    }

    #[test]
    fn matches_installed_java_constructor_defaults() {
        let configuration = Configuration {
            sdk_version: Some(" fixture-sdk-version ".into()),
            ..Default::default()
        };
        assert_eq!(configuration.to_wire_value(), fixtures()[0]["wire"]);
    }

    #[test]
    fn matches_installed_java_configured_model_and_json_bytes() {
        let fixture = &fixtures()[1];
        let entries = |key: &str| {
            fixture[key]
                .as_array()
                .unwrap()
                .iter()
                .map(|pair| (pair[0].as_str().map(str::to_owned), pair[1].clone()))
                .collect()
        };
        let configuration = Configuration {
            app_id: Some("\t fixture-app \r".into()),
            alternate_app_id: Some(" secondary ".into()),
            license: Some(" fixture/license ".into()),
            sdk_version: Some(" fixture-sdk-version ".into()),
            device_id: Some("\u{a0}device-fixture\u{a0}".into()),
            install_id: Some(" install-fixture ".into()),
            opaque_text: [
                Some(" opaque-1 ".into()),
                None,
                Some(" opaque-2 ".into()),
                Some(" opaque-3 ".into()),
            ],
            controls: [i32::MIN, i32::MAX],
            mode: -7,
            fallback_device_id: Some(" fallback-device ".into()),
            fallback_device_id_flag: 1,
            parameters: entries("parameters"),
            options: entries("options"),
        };
        assert_eq!(configuration.to_wire_value(), fixture["wire"]);
        assert_eq!(
            configuration.to_wire_json(),
            fixture["wireText"].as_str().unwrap()
        );
    }

    #[test]
    fn matches_public_native_network_bridge_fixtures() {
        // Ciphertext from classes48.dex; plaintext from the native library's
        // bounded offline probe. The error string exercises multiple cycles.
        let fixtures: [(&str, &[u8], &str); 7] = [
            ("3e6e04", &[42, 115, 81, 1], "http"),
            ("ae23f3", &[120, 115, 85, 87, 74], "https"),
            ("18c600", &[55, 41], "ws"),
            (
                "c7d21b",
                &[
                    123, 33, 87, 75, 27, 102, 116, 86, 55, 103, 50, 61, 3, 82, 30, 58, 104, 2, 33,
                    114, 97, 122, 0, 85, 29,
                ],
                "it must be http/https/wss",
            ),
            ("df2f3e", &[125, 112, 85, 2], "http"),
            ("166b73", &[55, 39], "ws"),
            ("23c455", &[22, 3, 60], "URL"),
        ];
        for (seed, payload, expected) in fixtures {
            assert_eq!(decode_constant(seed, payload).unwrap(), expected);
        }
    }

    #[test]
    fn rejects_missing_seed_and_invalid_utf8() {
        assert!(decode_constant("", &[42]).is_err());
        assert!(decode_constant("0", &[255]).is_err());
    }
}
