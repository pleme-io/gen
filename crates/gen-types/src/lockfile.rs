//! Typed [`Lockfile`] — resolved, content-addressed dependency tree.
//!
//! Every package manager ships a lockfile (`Cargo.lock`,
//! `package-lock.json`, `Gemfile.lock`, `poetry.lock`, `go.sum`,
//! `mix.lock`, …). Each pins exact resolved versions + integrity
//! hashes so subsequent installs are deterministic.
//!
//! Adapters parse their native lockfile into this canonical shape;
//! the resolver in `gen-engine` validates the manifest's typed
//! constraints against the lockfile's resolved versions.

use crate::{PackageId, PackageSource};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::fmt;

/// BLAKE3-32 content hash. Used for lockfile integrity + cache keys.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ContentHash(pub [u8; 32]);

impl ContentHash {
    /// Hash arbitrary bytes.
    #[must_use]
    pub fn of(bytes: &[u8]) -> Self {
        Self(*blake3::hash(bytes).as_bytes())
    }

    /// Genesis sentinel (all-zero).
    #[must_use]
    pub const fn genesis() -> Self {
        Self([0u8; 32])
    }

    /// 64-char lowercase hex.
    #[must_use]
    pub fn hex(&self) -> String {
        let mut s = String::with_capacity(64);
        for byte in &self.0 {
            s.push_str(&format!("{byte:02x}"));
        }
        s
    }

    /// Parse a 64-char hex string. Returns `None` for any other length
    /// or non-hex content.
    #[must_use]
    pub fn from_hex(s: &str) -> Option<Self> {
        if s.len() != 64 {
            return None;
        }
        let mut out = [0u8; 32];
        for (i, chunk) in s.as_bytes().chunks(2).enumerate() {
            let hex_str = std::str::from_utf8(chunk).ok()?;
            out[i] = u8::from_str_radix(hex_str, 16).ok()?;
        }
        Some(Self(out))
    }

    /// Parse a hex string of any length: 64 chars → BLAKE3-32, otherwise
    /// re-hash the raw string. Useful for ingesting heterogeneous
    /// integrity hashes (sha256:hex / sha512:hex / etc.) into the
    /// typed [`ContentHash`] cache-key space without losing identity.
    #[must_use]
    pub fn from_hex_padded(s: &str) -> Option<Self> {
        if s.len() == 64 {
            Self::from_hex(s)
        } else {
            Some(Self::of(s.as_bytes()))
        }
    }
}

impl fmt::Debug for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ContentHash({}…)", &self.hex()[..16])
    }
}

impl fmt::Display for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.hex())
    }
}

impl Serialize for ContentHash {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_str(&self.hex())
    }
}

impl<'de> Deserialize<'de> for ContentHash {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let s = String::deserialize(de)?;
        if s.len() != 64 {
            return Err(serde::de::Error::custom(format!(
                "ContentHash expected 64 hex chars, got {}",
                s.len()
            )));
        }
        let mut out = [0u8; 32];
        for (i, chunk) in s.as_bytes().chunks(2).enumerate() {
            let hex_str = std::str::from_utf8(chunk).map_err(serde::de::Error::custom)?;
            out[i] = u8::from_str_radix(hex_str, 16).map_err(serde::de::Error::custom)?;
        }
        Ok(Self(out))
    }
}

/// One resolved package in the lockfile. Pins exact source +
/// integrity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedPackage {
    pub id: PackageId,
    pub source: PackageSource,
    /// `sha256:…` / `sha512:…` / etc. from the registry.
    pub integrity: Option<String>,
    /// Resolved dependencies as edges to other `PackageId`s in the
    /// same lockfile.
    #[serde(default)]
    pub resolved_dependencies: Vec<PackageId>,
    /// `[package] links = "<symbol>"` from the package's `Cargo.toml`.
    /// Cargo sets `CARGO_MANIFEST_LINKS` from this value when invoking
    /// the build script. Build scripts for `ring`, the `*-sys` family
    /// (openssl-sys, bzip2-sys, libsqlite3-sys, …), and other native-
    /// shim crates ASSERT on this env var matching the declared
    /// `links`. Without this field threaded through the renderer the
    /// emitted Nix derivation builds the build-script binary with an
    /// empty `CARGO_MANIFEST_LINKS`, the assertion fires, and the
    /// crate fails to build. Populated by the consumer (gen-cli)
    /// from `Cargo.build-spec.json` at render time. Default-None so
    /// existing lockfiles stay deserialisable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub links: Option<String>,
}

/// Typed lockfile — content-addressed, deterministic.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lockfile {
    pub resolved: IndexMap<String, ResolvedPackage>,
    /// BLAKE3 of the canonical serialised lockfile. Cache layer keys
    /// on this for "did the workspace change?" gates.
    pub content_addressed_hash: ContentHash,
}

impl Lockfile {
    /// Empty lockfile (fresh project, never resolved). The
    /// content-addressed hash is the all-zero genesis.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            resolved: IndexMap::new(),
            content_addressed_hash: ContentHash::genesis(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Registry, Version};

    #[test]
    fn content_hash_round_trips_through_serde() {
        let h = ContentHash::of(b"some bytes");
        let j = serde_json::to_string(&h).unwrap();
        let parsed: ContentHash = serde_json::from_str(&j).unwrap();
        assert_eq!(h, parsed);
    }

    #[test]
    fn content_hash_genesis_is_all_zero() {
        assert_eq!(ContentHash::genesis().0, [0u8; 32]);
    }

    #[test]
    fn content_hash_hex_is_64_chars() {
        let h = ContentHash::of(b"x");
        assert_eq!(h.hex().len(), 64);
    }

    #[test]
    fn lockfile_round_trips_through_serde() {
        let mut resolved = IndexMap::new();
        resolved.insert(
            "serde".to_string(),
            ResolvedPackage {
                id: PackageId {
                    name: "serde".into(),
                    version: Version::new(1, 0, 228),
                    registry: Registry::CratesIo,
                },
                source: PackageSource::Registry {
                    registry: Registry::CratesIo,
                    registry_name: "serde".into(),
                    integrity_hash: Some("sha256:abc".into()),
                },
                integrity: Some("sha256:abc".into()),
                resolved_dependencies: vec![],
                links: None,
            },
        );
        let l = Lockfile {
            resolved,
            content_addressed_hash: ContentHash::of(b"snapshot"),
        };
        let j = serde_json::to_string(&l).unwrap();
        let parsed: Lockfile = serde_json::from_str(&j).unwrap();
        assert_eq!(l, parsed);
    }
}
