//! Typed [`Feature`] — named compose-vector of optional capabilities.
//!
//! Cargo has `[features]`; npm has nothing native (peerDeps the
//! closest analog); pip has `extras_require`; Composer has `suggest`;
//! gem-spec has groups.
//!
//! Adapters normalise to this shape; the resolver in `gen-engine`
//! computes the active feature set per-package from the consumer
//! tree.

use serde::{Deserialize, Serialize};

/// One named feature. Optional dependencies are expressed via the
/// implication graph: `feature X implies feature Y` + `feature Y is
/// "dep:some-optional-dep"`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Feature {
    pub name: String,
    /// Features this feature transitively enables (Cargo's `["foo",
    /// "bar/baz"]` shape).
    pub implies: Vec<FeatureRef>,
}

/// Reference to a feature by name. Either local (`"derive"` →
/// this package's `derive` feature) or namespaced (`"serde/derive"`
/// → another crate's feature).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum FeatureRef {
    /// Feature of this package.
    Local { name: String },
    /// Feature of a different package: `serde/derive`.
    Namespaced { package: String, feature: String },
    /// Pseudo-feature that activates an optional dependency
    /// (Cargo's `dep:foo` syntax).
    DepActivation { dep_name: String },
}

impl FeatureRef {
    /// Parse `serde/derive` / `dep:foo` / `derive` shapes into the
    /// matching typed variant.
    #[must_use]
    pub fn parse(s: &str) -> Self {
        if let Some(rest) = s.strip_prefix("dep:") {
            Self::DepActivation {
                dep_name: rest.to_string(),
            }
        } else if let Some((pkg, feat)) = s.split_once('/') {
            Self::Namespaced {
                package: pkg.to_string(),
                feature: feat.to_string(),
            }
        } else {
            Self::Local {
                name: s.to_string(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_local_feature() {
        assert_eq!(
            FeatureRef::parse("derive"),
            FeatureRef::Local {
                name: "derive".into()
            }
        );
    }

    #[test]
    fn parse_namespaced_feature() {
        assert_eq!(
            FeatureRef::parse("serde/derive"),
            FeatureRef::Namespaced {
                package: "serde".into(),
                feature: "derive".into(),
            }
        );
    }

    #[test]
    fn parse_dep_activation() {
        assert_eq!(
            FeatureRef::parse("dep:some-opt"),
            FeatureRef::DepActivation {
                dep_name: "some-opt".into()
            }
        );
    }

    #[test]
    fn round_trip_through_serde() {
        let f = Feature {
            name: "default".into(),
            implies: vec![
                FeatureRef::Local { name: "std".into() },
                FeatureRef::Namespaced {
                    package: "serde".into(),
                    feature: "derive".into(),
                },
            ],
        };
        let j = serde_json::to_string(&f).unwrap();
        let parsed: Feature = serde_json::from_str(&j).unwrap();
        assert_eq!(f, parsed);
    }
}
