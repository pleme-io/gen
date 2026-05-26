//! Typed [`Dependency`] — one edge in the package graph.

use crate::{TargetPredicate, VersionConstraint};
use serde::{Deserialize, Serialize};

/// One dependency edge from a parent package to a child.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dependency {
    pub name: String,
    pub constraint: VersionConstraint,
    pub kind: DependencyKind,
    /// Which of the dependency's features the parent enables.
    #[serde(default)]
    pub features_enabled: Vec<String>,
    /// Whether to pull the dependency's `default` feature set.
    #[serde(default = "default_true")]
    pub default_features: bool,
    /// Conditional-activation predicate (cfg / engine / platform).
    #[serde(default)]
    pub target_predicate: Option<TargetPredicate>,
    /// `[patch."<registry>"]` / `npm overrides` — a per-edge source
    /// override. Most edges have None; resolver consults this when
    /// non-None.
    #[serde(default)]
    pub source_override: Option<crate::PackageSource>,
}

fn default_true() -> bool {
    true
}

/// Typed kind covering every per-language dep classification.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DependencyKind {
    /// Standard runtime dependency.
    Direct,
    /// Build-time only (cargo `[build-dependencies]`, npm absent —
    /// approximated as devDeps used during build script).
    Build,
    /// Test / dev only (cargo `[dev-dependencies]`, npm
    /// `devDependencies`, gem `:development`, pip `[dev]` extras).
    Dev,
    /// Optional — pulled in by feature activation (cargo
    /// `optional = true`, npm `optionalDependencies`).
    Optional,
    /// Peer dependency (npm `peerDependencies`) — declared by the
    /// consumer, satisfied by the parent. Cargo has no direct
    /// analog; adapters can use this for similar "consumer
    /// provides" relationships.
    Peer,
    /// Replaces / substitutes another package (cargo `[replace]`,
    /// npm `overrides`, Bundler `gemspec` source overrides).
    Replaces,
}

impl DependencyKind {
    /// Stable string token for diagnostics + cache keys.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Build => "build",
            Self::Dev => "dev",
            Self::Optional => "optional",
            Self::Peer => "peer",
            Self::Replaces => "replaces",
        }
    }

    /// Is this dependency kind needed at runtime?
    /// (used by the resolver to filter dev/build deps for
    /// release-build cache hits).
    #[must_use]
    pub const fn is_runtime(self) -> bool {
        matches!(self, Self::Direct | Self::Optional | Self::Peer | Self::Replaces)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ConstraintSpec, Version};

    #[test]
    fn kind_string_tokens_are_stable() {
        for k in [
            DependencyKind::Direct,
            DependencyKind::Build,
            DependencyKind::Dev,
            DependencyKind::Optional,
            DependencyKind::Peer,
            DependencyKind::Replaces,
        ] {
            assert!(!k.as_str().is_empty());
        }
    }

    #[test]
    fn is_runtime_classifies_correctly() {
        assert!(DependencyKind::Direct.is_runtime());
        assert!(DependencyKind::Optional.is_runtime());
        assert!(DependencyKind::Peer.is_runtime());
        assert!(DependencyKind::Replaces.is_runtime());
        assert!(!DependencyKind::Build.is_runtime());
        assert!(!DependencyKind::Dev.is_runtime());
    }

    #[test]
    fn round_trip_through_serde() {
        let d = Dependency {
            name: "serde".into(),
            constraint: VersionConstraint::from_spec(ConstraintSpec::Caret(Version::new(1, 0, 0))),
            kind: DependencyKind::Direct,
            features_enabled: vec!["derive".into()],
            default_features: true,
            target_predicate: None,
            source_override: None,
        };
        let j = serde_json::to_string(&d).unwrap();
        let parsed: Dependency = serde_json::from_str(&j).unwrap();
        assert_eq!(d, parsed);
    }
}
