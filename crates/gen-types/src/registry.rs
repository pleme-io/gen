//! Typed [`Registry`] — where a package was fetched from.
//!
//! One variant per upstream registry the fleet's adapters speak to.
//! Adapters set the variant during parse; renderers consult it to
//! emit the correct download URL / cache key.

use serde::{Deserialize, Serialize};

/// Stable identifier for the upstream package registry.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Registry {
    /// crates.io (default for Cargo).
    CratesIo,
    /// npmjs.org (default for npm / yarn / pnpm).
    Npm,
    /// rubygems.org (default for Bundler / Gem).
    RubyGems,
    /// pypi.org (default for pip / poetry).
    PyPi,
    /// proxy.golang.org (default for go modules).
    GoProxy,
    /// hex.pm (default for Mix / Hex).
    Hex,
    /// hackage.haskell.org (default for Cabal).
    Hackage,
    /// packagist.org (default for Composer).
    Packagist,
    /// Maven Central + Gradle plugin portal.
    Maven,
    /// pub.dev (Dart / Flutter).
    Pub,
    /// OCI registry (helm charts via OCI, also caixa).
    Oci { registry_url: String },
    /// Operator-self-hosted private registry — adapters declare the
    /// URL + auth shape via shikumi config.
    Private {
        url: String,
        protocol: String,
    },
    /// Source-only — no registry; the package is consumed as a path
    /// or git ref, never published.
    None,
}

impl Registry {
    /// Stable short identifier (for cache keys + diagnostics).
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::CratesIo => "crates-io",
            Self::Npm => "npm",
            Self::RubyGems => "rubygems",
            Self::PyPi => "pypi",
            Self::GoProxy => "go-proxy",
            Self::Hex => "hex",
            Self::Hackage => "hackage",
            Self::Packagist => "packagist",
            Self::Maven => "maven",
            Self::Pub => "pub",
            Self::Oci { .. } => "oci",
            Self::Private { .. } => "private",
            Self::None => "none",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn as_str_covers_every_variant_stably() {
        for r in [
            Registry::CratesIo,
            Registry::Npm,
            Registry::RubyGems,
            Registry::PyPi,
            Registry::GoProxy,
            Registry::Hex,
            Registry::Hackage,
            Registry::Packagist,
            Registry::Maven,
            Registry::Pub,
            Registry::Oci {
                registry_url: "ghcr.io".into(),
            },
            Registry::Private {
                url: "https://internal".into(),
                protocol: "cargo-v1".into(),
            },
            Registry::None,
        ] {
            assert!(!r.as_str().is_empty());
        }
    }

    #[test]
    fn round_trips_through_serde() {
        let r = Registry::Oci {
            registry_url: "ghcr.io/pleme-io".into(),
        };
        let j = serde_json::to_string(&r).unwrap();
        let parsed: Registry = serde_json::from_str(&j).unwrap();
        assert_eq!(r, parsed);
    }
}
