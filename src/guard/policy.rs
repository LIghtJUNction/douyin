//! Path-rule stage of the inspected 40.8.0 SDK guard selector.
//! Effective settings, host/account gates, timestamps and key availability must
//! be established separately. This module does not authorize or sign a request.

/// Literal, case-sensitive SDK path rules; do not URL-decode before matching.
#[derive(Clone, Default)]
pub struct PathRules {
    pub paths: Vec<String>,
    pub prefixes: Vec<String>,
    pub excluded_paths: Vec<String>,
    pub excluded_prefixes: Vec<String>,
}

impl PathRules {
    fn matches(&self, path: &str, forced: &[String]) -> bool {
        if forced.iter().any(|item| item == path) {
            return true;
        }
        if self.excluded_paths.iter().any(|item| item == path)
            || self
                .excluded_prefixes
                .iter()
                .any(|item| path.starts_with(item))
        {
            return false;
        }
        self.paths.iter().any(|item| item == path)
            || self.prefixes.iter().any(|item| path.starts_with(item))
    }
}

/// Explicit effective settings, not a set of guessed runtime defaults.
/// Resolve each SDK setting separately: overrides apply only at selected load
/// sites, and an explicit empty array is different from a missing array.
/// HMAC and REE share ree_force_sign_paths in the inspected SDK.
pub struct GuardPolicy {
    pub separate_tee_ree: bool,
    pub legacy: PathRules,
    pub tee: PathRules,
    pub ree: PathRules,
    pub ree_force_sign_paths: Vec<String>,
    pub hmac_enabled: bool,
    pub hmac: PathRules,
}

/// Independent requests from the path-rule stage, not an availability fallback.
/// A requested TEE operation must not be replaced merely because HMAC is present.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GuardPlan {
    pub tee: bool,
    pub ree: bool,
    pub hmac: bool,
}

impl GuardPolicy {
    pub fn select(&self, path: &str) -> GuardPlan {
        let (tee, ree) = if self.separate_tee_ree {
            (
                self.tee.matches(path, &[]),
                self.ree.matches(path, &self.ree_force_sign_paths),
            )
        } else {
            let legacy = self.legacy.matches(path, &[]);
            (legacy, legacy)
        };
        GuardPlan {
            tee,
            ree,
            hmac: self.hmac_enabled && self.hmac.matches(path, &self.ree_force_sign_paths),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> GuardPolicy {
        GuardPolicy {
            separate_tee_ree: true,
            legacy: PathRules {
                prefixes: vec!["/legacy/".into()],
                ..Default::default()
            },
            tee: PathRules {
                paths: vec!["/tee".into(), "/both".into()],
                ..Default::default()
            },
            ree: PathRules {
                paths: vec!["/ree".into(), "/both".into()],
                ..Default::default()
            },
            ree_force_sign_paths: vec!["/forced".into()],
            hmac_enabled: true,
            hmac: PathRules::default(),
        }
    }

    #[test]
    fn separate_and_legacy_rules_preserve_both_hardware_and_software_requests() {
        let mut p = policy();
        for (path, tee, ree) in [
            ("/none", false, false),
            ("/tee", true, false),
            ("/ree", false, true),
            ("/both", true, true),
        ] {
            assert_eq!(
                p.select(path),
                GuardPlan {
                    tee,
                    ree,
                    hmac: false
                }
            );
        }
        p.separate_tee_ree = false;
        assert_eq!(
            p.select("/legacy/a"),
            GuardPlan {
                tee: true,
                ree: true,
                hmac: false
            }
        );
        assert_eq!(
            p.select("/both"),
            GuardPlan {
                tee: false,
                ree: false,
                hmac: false
            }
        );
        assert_eq!(
            p.select("/forced"),
            GuardPlan {
                tee: false,
                ree: false,
                hmac: true
            }
        );
    }

    #[test]
    fn force_precedes_exclusions_but_does_not_enable_hmac_or_override_tee_rules() {
        let mut p = policy();
        p.tee.excluded_paths.push("/forced".into());
        p.ree.excluded_prefixes.push("/".into());
        p.hmac.excluded_paths.push("/forced".into());
        assert_eq!(
            p.select("/forced"),
            GuardPlan {
                tee: false,
                ree: true,
                hmac: true
            }
        );
        p.hmac_enabled = false;
        assert!(!p.select("/forced").hmac);
        assert!(!p.select("/ree").ree);
        p.tee.excluded_paths.push("/tee".into());
        assert!(!p.select("/tee").tee);
    }

    #[test]
    fn prefix_matching_is_literal_and_exclusions_win_over_ordinary_inclusions() {
        let rules = PathRules {
            prefixes: vec!["/a/".into()],
            excluded_prefixes: vec!["/a/private".into()],
            paths: vec!["/a/private/item".into()],
            ..Default::default()
        };
        assert!(rules.matches("/a/item", &[]));
        assert!(!rules.matches("/a/private/item", &[]));
        assert!(!rules.matches("/A/item", &[]));
        assert!(!rules.matches("/a%2Fitem", &[]));
        assert!(!rules.matches("/ab/item", &[]));
    }
}
