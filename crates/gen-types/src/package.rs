//! Typed [`Package`] — one node in the dependency graph.

use crate::{BuildStep, Dependency, Feature, PackageSource, Registry, Version};
use serde::{Deserialize, Serialize};

/// Canonical identifier: `(name, version, registry)`. Different
/// registries can host packages with the same name+version, so the
/// registry is part of identity.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PackageId {
    pub name: String,
    pub version: Version,
    pub registry: Registry,
}

impl PackageId {
    /// Stable string form for diagnostics + filesystem-safe cache
    /// keys: `<registry>/<name>/<version>`.
    #[must_use]
    pub fn dotted(&self) -> String {
        format!("{}/{}/{}", self.registry.as_str(), self.name, self.version)
    }
}

/// One package in the parsed manifest. Adapter-agnostic shape;
/// every language reduces to this.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Package {
    pub name: String,
    pub version: Version,
    pub source: PackageSource,
    pub registry: Registry,
    #[serde(default)]
    pub dependencies: Vec<Dependency>,
    #[serde(default)]
    pub features: Vec<Feature>,
    #[serde(default)]
    pub build_steps: Vec<BuildStep>,
    /// SPDX license string when available.
    #[serde(default)]
    pub license: Option<String>,
    /// Free-text description from the manifest.
    #[serde(default)]
    pub description: Option<String>,
    /// Authors / publisher line.
    #[serde(default)]
    pub authors: Vec<String>,
    /// Homepage URL.
    #[serde(default)]
    pub homepage: Option<String>,
    /// Source repository URL.
    #[serde(default)]
    pub repository: Option<String>,
}

impl Package {
    /// Convenience: canonical identity of this package.
    #[must_use]
    pub fn id(&self) -> PackageId {
        PackageId {
            name: self.name.clone(),
            version: self.version.clone(),
            registry: self.registry.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ConstraintSpec;

    #[test]
    fn id_dotted_form_is_stable() {
        let p = Package {
            name: "serde".into(),
            version: Version::new(1, 0, 228),
            source: PackageSource::Registry {
                registry: Registry::CratesIo,
                registry_name: "serde".into(),
                integrity_hash: None,
            },
            registry: Registry::CratesIo,
            dependencies: vec![],
            features: vec![],
            build_steps: vec![],
            license: Some("MIT OR Apache-2.0".into()),
            description: None,
            authors: vec![],
            homepage: None,
            repository: None,
        };
        assert_eq!(p.id().dotted(), "crates-io/serde/1.0.228");
    }

    #[test]
    fn package_id_used_as_hash_key() {
        let id_a = PackageId {
            name: "serde".into(),
            version: Version::new(1, 0, 0),
            registry: Registry::CratesIo,
        };
        let id_b = id_a.clone();
        let mut m = std::collections::HashMap::new();
        m.insert(id_a, "x");
        assert_eq!(m.get(&id_b), Some(&"x"));
    }

    #[test]
    fn round_trip_through_serde() {
        let p = Package {
            name: "serde".into(),
            version: Version::new(1, 0, 228),
            source: PackageSource::Registry {
                registry: Registry::CratesIo,
                registry_name: "serde".into(),
                integrity_hash: Some("sha256:abc".into()),
            },
            registry: Registry::CratesIo,
            dependencies: vec![Dependency {
                name: "serde_derive".into(),
                constraint: crate::VersionConstraint::from_spec(ConstraintSpec::Caret(
                    Version::new(1, 0, 228),
                )),
                kind: crate::DependencyKind::Direct,
                features_enabled: vec![],
                default_features: true,
                target_predicate: None,
                source_override: None,
            }],
            features: vec![],
            build_steps: vec![],
            license: Some("MIT".into()),
            description: Some("Serialization framework".into()),
            authors: vec![],
            homepage: None,
            repository: Some("https://github.com/serde-rs/serde".into()),
        };
        let j = serde_json::to_string(&p).unwrap();
        let parsed: Package = serde_json::from_str(&j).unwrap();
        assert_eq!(p, parsed);
    }
}
