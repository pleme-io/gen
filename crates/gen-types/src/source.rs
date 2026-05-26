//! Typed [`PackageSource`] — where a package's bits come from.

use crate::Registry;
use serde::{Deserialize, Serialize};

/// One typed source variant per resolution kind every adapter
/// surfaces.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum PackageSource {
    /// Fetched from an upstream registry (most common case).
    Registry {
        registry: Registry,
        /// Canonical name in the registry's namespace (may differ
        /// from `Package.name` for renames / aliases).
        registry_name: String,
        /// SHA-256 / BLAKE3 / etc. of the registry tarball — the
        /// content-addressed integrity check the lockfile pins.
        integrity_hash: Option<String>,
    },
    /// Fetched from a git repository.
    Git {
        url: String,
        /// Branch / tag / commit-rev — adapters normalise to commit-
        /// rev during resolution so the lockfile is reproducible.
        rev: String,
        /// Subdirectory inside the git tree, if the package isn't at
        /// the repo root.
        #[serde(default)]
        subdir: Option<String>,
    },
    /// Local path (workspace-relative or absolute).
    Path { path: String },
    /// Local development override (`[patch."<registry>"]` in Cargo;
    /// `npm install file:../foo`; etc.). Differs from `Path` in that
    /// it's an override of a registry source, not the canonical
    /// source.
    Local { path: String, overrides: String },
}

impl PackageSource {
    /// Stable string identifier for cache keys + diagnostics.
    #[must_use]
    pub const fn kind_str(&self) -> &'static str {
        match self {
            Self::Registry { .. } => "registry",
            Self::Git { .. } => "git",
            Self::Path { .. } => "path",
            Self::Local { .. } => "local",
        }
    }

    /// Is this source content-addressed (registry with integrity hash
    /// OR git with a commit-rev)? Cache-hit-first only makes sense
    /// for content-addressed sources — path + local sources rebuild
    /// every time.
    #[must_use]
    pub fn is_content_addressed(&self) -> bool {
        match self {
            Self::Registry { integrity_hash, .. } => integrity_hash.is_some(),
            Self::Git { .. } => true,
            Self::Path { .. } | Self::Local { .. } => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_with_hash_is_content_addressed() {
        let s = PackageSource::Registry {
            registry: Registry::CratesIo,
            registry_name: "serde".into(),
            integrity_hash: Some("sha256:abc".into()),
        };
        assert!(s.is_content_addressed());
    }

    #[test]
    fn registry_without_hash_is_not_content_addressed() {
        let s = PackageSource::Registry {
            registry: Registry::CratesIo,
            registry_name: "serde".into(),
            integrity_hash: None,
        };
        assert!(!s.is_content_addressed());
    }

    #[test]
    fn git_is_always_content_addressed() {
        let s = PackageSource::Git {
            url: "https://github.com/x/y".into(),
            rev: "abc123".into(),
            subdir: None,
        };
        assert!(s.is_content_addressed());
    }

    #[test]
    fn path_and_local_are_not_content_addressed() {
        assert!(!PackageSource::Path { path: "../foo".into() }.is_content_addressed());
        assert!(
            !PackageSource::Local {
                path: "../foo".into(),
                overrides: "crates-io.serde".into(),
            }
            .is_content_addressed()
        );
    }

    #[test]
    fn round_trip_through_serde() {
        let s = PackageSource::Git {
            url: "https://github.com/x/y".into(),
            rev: "abc123".into(),
            subdir: Some("crates/foo".into()),
        };
        let j = serde_json::to_string(&s).unwrap();
        let parsed: PackageSource = serde_json::from_str(&j).unwrap();
        assert_eq!(s, parsed);
    }
}
