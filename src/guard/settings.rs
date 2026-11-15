//! Setting layers from X.1Yp2 and X.1YZm, before path policy selection.
//! Callers must supply the verified constructor default and current local value.
use std::collections::BTreeSet;

use serde_json::{Map, Value};

/// SDK array layers after JSONArray.optString conversion. JSON coercion is not
/// performed here: callers must supply the SDK-decoded strings, not raw values.
/// A deterministic order is used because SDK membership checks ignore set order.
pub struct ArraySetting {
    default: Vec<String>,
    additions: Vec<String>,
    base: Vec<String>,
    override_value: Option<Vec<String>>,
}

impl ArraySetting {
    pub fn new(default: Vec<String>, additions: Vec<String>) -> Self {
        Self {
            base: default.clone(),
            default,
            additions,
            override_value: None,
        }
    }

    /// Missing arrays restore the constructor default. Explicit [] clears the
    /// base layer, but does not erase additions or an existing override.
    pub fn update_base(&mut self, value: Option<Vec<String>>) {
        self.base = value
            .map(decoded_array)
            .unwrap_or_else(|| self.default.clone());
    }

    /// Call only at SDK load sites that support overrides. Some path settings
    /// never read an override, even if the same key appears in the JSON object.
    pub fn update_override(&mut self, value: Option<Vec<String>>) {
        self.override_value = value.map(decoded_array);
    }

    pub fn effective(&self) -> Vec<String> {
        let values: BTreeSet<_> = match &self.override_value {
            Some(value) => value.iter().cloned().collect(),
            None => self.additions.iter().chain(&self.base).cloned().collect(),
        };
        values.into_iter().collect()
    }
}

fn decoded_array(value: Vec<String>) -> Vec<String> {
    // TextUtils.isEmpty, not trim: whitespace-containing rules remain literal.
    value.into_iter().filter(|item| !item.is_empty()).collect()
}

pub struct BooleanSetting {
    default: bool,
    base: bool,
    local: Option<bool>,
    override_value: Option<bool>,
}

impl BooleanSetting {
    pub fn new(default: bool) -> Self {
        Self {
            default,
            base: default,
            local: None,
            override_value: None,
        }
    }

    pub fn set_local(&mut self, value: Option<bool>) {
        self.local = value;
    }

    /// Mirror the SDK Boolean loader. An absent base object retains its previous
    /// value; a present base object with a missing key restores the SDK default.
    /// A missing/null override clears the override, rather than retaining it.
    pub fn update(
        &mut self,
        key: &str,
        base: Option<&Map<String, Value>>,
        overrides: Option<&Map<String, Value>>,
    ) {
        self.override_value = overrides
            .and_then(|values| values.get(key))
            .filter(|value| !value.is_null())
            .map(|value| android_boolean(Some(value), self.default));
        if let Some(base) = base {
            self.base = android_boolean(base.get(key), self.default);
        }
    }

    pub fn effective(&self) -> bool {
        self.override_value.or(self.local).unwrap_or(self.base)
    }
}

// Android libcore JSON.toBoolean uses Boolean or String.equalsIgnoreCase,
// without trimming or numeric truthiness. Java's ASCII-letter case equivalence
// also includes U+017F (long s), which can occur in "false".
// https://android.googlesource.com/platform/libcore/+/refs/heads/android16-release/json/src/main/java/org/json/JSON.java
fn android_boolean(value: Option<&Value>, fallback: bool) -> bool {
    match value {
        Some(Value::Bool(value)) => *value,
        Some(Value::String(value)) if value.eq_ignore_ascii_case("true") => true,
        Some(Value::String(value))
            if value.replace('\u{017f}', "s").eq_ignore_ascii_case("false") =>
        {
            false
        }
        _ => fallback,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn empty_base_retains_additions_while_missing_base_restores_defaults() {
        let mut setting = ArraySetting::new(vec!["/default".into()], vec!["/local".into()]);
        assert_eq!(setting.effective(), vec!["/default", "/local"]);
        setting.update_base(Some(vec!["/remote".into()]));
        assert_eq!(setting.effective(), vec!["/local", "/remote"]);
        setting.update_base(Some(vec![]));
        assert_eq!(setting.effective(), vec!["/local"]);
        setting.update_base(None);
        assert_eq!(setting.effective(), vec!["/default", "/local"]);
    }

    #[test]
    fn empty_override_clears_every_layer_and_removing_it_restores_current_base() {
        let mut setting = ArraySetting::new(vec!["/default".into()], vec!["/local".into()]);
        setting.update_override(Some(vec![]));
        assert!(setting.effective().is_empty());
        setting.update_base(Some(vec!["/new".into()]));
        assert!(setting.effective().is_empty());
        setting.update_override(Some(vec!["/override".into()]));
        assert_eq!(setting.effective(), vec!["/override"]);
        setting.update_override(None);
        assert_eq!(setting.effective(), vec!["/local", "/new"]);
    }

    #[test]
    fn decoded_arrays_deduplicate_without_trimming_or_normalizing_paths() {
        let mut setting = ArraySetting::new(vec![], vec![]);
        setting.update_base(Some(vec![
            "".into(),
            "/a%2Fb".into(),
            "/a%2Fb".into(),
            " /a ".into(),
        ]));
        assert_eq!(setting.effective(), vec![" /a ", "/a%2Fb"]);
        setting.update_override(Some(vec!["".into()]));
        assert!(setting.effective().is_empty());
    }

    #[test]
    fn resolved_boolean_and_array_layers_drive_policy_without_erasing_tee_requirements() {
        use crate::guard::policy::{GuardPlan, GuardPolicy, PathRules};
        let mut enabled = BooleanSetting::new(false);
        enabled.update("enable", json!({"enable":"TRUE"}).as_object(), None);
        let mut paths = ArraySetting::new(vec!["/live/".into()], vec![]);
        paths.update_override(Some(vec![]));
        let mut policy = GuardPolicy {
            separate_tee_ree: true,
            legacy: PathRules::default(),
            tee: PathRules {
                paths: vec!["/live/create".into()],
                ..Default::default()
            },
            ree: PathRules::default(),
            ree_force_sign_paths: vec![],
            hmac_enabled: enabled.effective(),
            hmac: PathRules {
                prefixes: paths.effective(),
                ..Default::default()
            },
        };
        assert_eq!(
            policy.select("/live/create"),
            GuardPlan {
                tee: true,
                ree: false,
                hmac: false
            }
        );
        paths.update_override(None);
        policy.hmac.prefixes = paths.effective();
        assert_eq!(
            policy.select("/live/create"),
            GuardPlan {
                tee: true,
                ree: false,
                hmac: true
            }
        );
    }

    #[test]
    fn boolean_conversion_matches_android_strings_without_trimming_or_truthiness() {
        for (input, result) in [
            (json!(true), true),
            (json!(false), false),
            (json!("TrUe"), true),
            (json!("FaLſE"), false),
        ] {
            for fallback in [false, true] {
                assert_eq!(android_boolean(Some(&input), fallback), result);
            }
        }
        for input in [
            json!(null),
            json!(1),
            json!(0),
            json!([]),
            json!({}),
            json!(" true"),
            json!("false "),
            json!("1"),
            json!("yes"),
        ] {
            for fallback in [false, true] {
                assert_eq!(android_boolean(Some(&input), fallback), fallback);
            }
        }
    }

    #[test]
    fn invalid_nonnull_override_uses_default_but_still_overrides_local_and_base() {
        let mut setting = BooleanSetting::new(true);
        setting.set_local(Some(false));
        let base = json!({"flag":false});
        let bad_override = json!({"flag":"invalid"});
        setting.update("flag", base.as_object(), bad_override.as_object());
        assert!(setting.effective());
        setting.update("flag", base.as_object(), json!({"flag":null}).as_object());
        assert!(!setting.effective());
        setting.set_local(None);
        assert!(!setting.effective());
    }

    #[test]
    fn absent_object_retains_base_but_absent_key_restores_default_and_clears_override() {
        let mut setting = BooleanSetting::new(false);
        setting.update("flag", json!({"flag":true}).as_object(), None);
        assert!(setting.effective());
        setting.update("flag", None, json!({"flag":false}).as_object());
        assert!(!setting.effective());
        setting.update("flag", None, None);
        assert!(setting.effective());
        setting.update("flag", json!({}).as_object(), None);
        assert!(!setting.effective());
    }
}
