//! `gen-gomod` slim resolver-delta — `Go.gen.lock`.
//!
//! The gomod analogue of `Cargo.gen.lock`. It carries ONLY the facts a
//! Go module's `go.mod` + `go.sum` cannot express on their own —
//! principally the per-package `vendorHash` (the NAR-sha256 of the
//! resolved dependency tree, which neither go.mod nor go.sum contains)
//! plus the build-shaping scalars (tags, ldflags, sub-packages,
//! quirks). Everything go.mod/go.sum already pin (module path, dep
//! versions, dep hashes) is reconstructed by the substrate consumer in
//! pure Nix via `builtins.readFile` — never restated here.
//!
//! Freshness tie: `go_sum_sha256` — the lowercase-hex SHA-256 of
//! `go.sum`. The consumer compares it to `builtins.hashFile "sha256"`
//! of the on-disk go.sum; a match means the committed `vendorHash` is
//! still valid (the dep set is unchanged). This mirrors cargo's
//! `cargo_lock_sha256` D2 tie.
//!
//! Implements [`gen_types::GenDeltaArtifact`] — the SAME trait gen-cargo
//! re-exports — so the delta shape is uniform across ecosystems.

use std::path::Path;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::build_spec::BuildSpec;
use crate::quirks::GomodQuirk;

pub use gen_types::GenDeltaArtifact;

/// Schema version of the slim delta artifact. Distinct from
/// `build_spec::SCHEMA_VERSION` (the full spec's version).
pub const DELTA_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Error)]
pub enum GoGenDeltaError {
    #[error("gen-delta: refusing to emit a delta with zero packages")]
    EmptyPackages,
    #[error("gen-delta: go.sum not found at {0} — the freshness tie is mandatory")]
    NoGoSum(std::path::PathBuf),
    #[error("gen-delta: read go.sum at {path}: {source}")]
    ReadGoSum {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("gen-delta: serialize Go.gen.lock: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("gen-delta: write {path}: {source}")]
    Write {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

/// The resolver-only, per-package facts the lockfile can't express.
/// Every field here is the gomod analogue of cargo's `PerCrateScalars`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PerPackageDelta {
    /// The resolved module path (e.g. `github.com/example/widget`).
    pub module: String,
    /// The NAR-sha256 `vendorHash` (SRI `sha256-<base64>`). `None` when
    /// the module has no external deps (nixpkgs `vendorHash = null`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vendor_hash: Option<String>,
    /// `go build -tags …` entries.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// `go build -ldflags …` entries.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ldflags: Vec<String>,
    /// Subpackages to build (paths relative to module root).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sub_packages: Vec<String>,
    /// Typed quirks for this package.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quirks: Vec<GomodQuirk>,
}

/// `Go.gen.lock` — the slim resolver delta.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GoGenDelta {
    pub schema_version: u32,
    /// Freshness tie. Lowercase hex SHA-256 of `go.sum`.
    pub go_sum_sha256: String,
    /// Per-package deltas, keyed by the same key as `BuildSpec.packages`.
    pub per_package: IndexMap<String, PerPackageDelta>,
}

impl GoGenDelta {
    /// Distill the delta from the full spec given the go.sum hash (the
    /// freshness tie). Separated from the trait `distill` so the hash
    /// can be threaded from the file at write time without re-reading.
    fn distill_with_sum(spec: &BuildSpec, go_sum_sha256: String) -> Result<Self, GoGenDeltaError> {
        if spec.packages.is_empty() {
            return Err(GoGenDeltaError::EmptyPackages);
        }
        let per_package: IndexMap<String, PerPackageDelta> = spec
            .packages
            .iter()
            .map(|(key, p)| {
                (
                    key.clone(),
                    PerPackageDelta {
                        module: p.name.clone(),
                        vendor_hash: p.args.vendor_hash.clone(),
                        tags: p.args.tags.clone(),
                        ldflags: p.args.ldflags.clone(),
                        sub_packages: p.args.sub_packages.clone(),
                        quirks: p.quirks.clone(),
                    },
                )
            })
            .collect();
        Ok(GoGenDelta {
            schema_version: DELTA_SCHEMA_VERSION,
            go_sum_sha256,
            per_package,
        })
    }

    /// Serialize to pretty JSON (deterministic via `IndexMap` ordering).
    pub fn to_json(&self) -> Result<String, GoGenDeltaError> {
        Ok(serde_json::to_string_pretty(self)?)
    }
}

impl GenDeltaArtifact for GoGenDelta {
    type FullSpec = BuildSpec;
    type Error = GoGenDeltaError;
    const SCHEMA_VERSION: u32 = DELTA_SCHEMA_VERSION;
    const FILENAME: &'static str = "Go.gen.lock";

    /// Trait `distill` — for a module with no go.sum (a leaf module
    /// with zero external deps), the freshness tie is the hash of the
    /// empty string. The richer `distill_with_sum` is used by
    /// `write_gen_delta` to thread the real on-disk hash.
    fn distill(spec: &BuildSpec) -> Result<Self, GoGenDeltaError> {
        Self::distill_with_sum(spec, sha256_hex(b""))
    }

    fn lock_sha256(&self) -> &str {
        &self.go_sum_sha256
    }
}

/// Lowercase-hex SHA-256 of bytes — matches `builtins.hashFile
/// "sha256"` on the Nix side.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    let digest = h.finalize();
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Distill + write `Go.gen.lock` next to the module's `go.sum`. Reads
/// the on-disk `go.sum` for the freshness tie (empty-string hash when
/// the module is dependency-free and has no go.sum). Additive — call
/// after the full build-spec write. Propagates errors: a failed delta
/// emit MUST fail `gen build`.
pub fn write_gen_delta(root: &Path, spec: &BuildSpec) -> Result<(), GoGenDeltaError> {
    let go_sum_path = root.join("go.sum");
    let go_sum_sha256 = match std::fs::read(&go_sum_path) {
        Ok(bytes) => sha256_hex(&bytes),
        // A dependency-free module legitimately has no go.sum; tie to
        // the empty-string hash rather than failing.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => sha256_hex(b""),
        Err(source) => {
            return Err(GoGenDeltaError::ReadGoSum {
                path: go_sum_path.display().to_string(),
                source,
            });
        }
    };
    let delta = GoGenDelta::distill_with_sum(spec, go_sum_sha256)?;
    let path = root.join(GoGenDelta::FILENAME);
    std::fs::write(&path, delta.to_json()? + "\n").map_err(|source| GoGenDeltaError::Write {
        path: path.display().to_string(),
        source,
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_spec::{PackageArgs, PackageSpec};

    fn fixture_spec() -> BuildSpec {
        let mut packages = IndexMap::new();
        packages.insert(
            "widget".to_string(),
            PackageSpec {
                name: "github.com/example/widget".to_string(),
                version: "0.0.0".to_string(),
                args: PackageArgs {
                    pname: Some("widget".into()),
                    version: Some("0.0.0".into()),
                    vendor_hash: Some("sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".into()),
                    proxy_vendor: Some(true),
                    tags: vec!["netgo".into()],
                    ldflags: vec!["-s".into(), "-w".into()],
                    sub_packages: vec!["cmd/widget".into()],
                    ..Default::default()
                },
                has_external_deps: true,
                quirks: vec![GomodQuirk::CgoOff],
            },
        );
        BuildSpec {
            version: crate::build_spec::SCHEMA_VERSION,
            packages,
            root_package: "widget".to_string(),
            workspace_members: vec!["github.com/example/widget".to_string()],
        }
    }

    #[test]
    fn filename_is_go_gen_lock() {
        assert_eq!(GoGenDelta::FILENAME, "Go.gen.lock");
    }

    #[test]
    fn distill_carries_vendor_hash_tags_ldflags_quirks() {
        let d = GoGenDelta::distill(&fixture_spec()).unwrap();
        let p = &d.per_package["widget"];
        assert_eq!(
            p.vendor_hash.as_deref(),
            Some("sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=")
        );
        assert_eq!(p.tags, vec!["netgo"]);
        assert_eq!(p.ldflags, vec!["-s", "-w"]);
        assert_eq!(p.sub_packages, vec!["cmd/widget"]);
        assert_eq!(p.quirks, vec![GomodQuirk::CgoOff]);
        assert_eq!(p.module, "github.com/example/widget");
    }

    #[test]
    fn freshness_tie_is_lowercase_hex_sha256() {
        let d = GoGenDelta::distill(&fixture_spec()).unwrap();
        let s = d.lock_sha256();
        assert_eq!(s.len(), 64, "sha256 hex is 64 chars");
        assert!(s
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    #[test]
    fn empty_packages_rejected() {
        let spec = BuildSpec {
            version: crate::build_spec::SCHEMA_VERSION,
            packages: IndexMap::new(),
            root_package: String::new(),
            workspace_members: Vec::new(),
        };
        assert!(matches!(
            GoGenDelta::distill(&spec),
            Err(GoGenDeltaError::EmptyPackages)
        ));
    }

    #[test]
    fn roundtrip_is_lossless() {
        let d = GoGenDelta::distill(&fixture_spec()).unwrap();
        let json = d.to_json().unwrap();
        let back: GoGenDelta = serde_json::from_str(&json).unwrap();
        assert_eq!(json, back.to_json().unwrap());
    }

    #[test]
    fn write_emits_go_gen_lock_with_real_sum_tie() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::write(root.join("go.sum"), b"example.com/x v1.0.0 h1:abc=\n").unwrap();
        write_gen_delta(root, &fixture_spec()).unwrap();
        let written = std::fs::read_to_string(root.join("Go.gen.lock")).unwrap();
        let back: GoGenDelta = serde_json::from_str(&written).unwrap();
        // The freshness tie equals the SHA-256 of the on-disk go.sum.
        let expected = sha256_hex(b"example.com/x v1.0.0 h1:abc=\n");
        assert_eq!(back.go_sum_sha256, expected);
    }

    #[test]
    fn write_handles_missing_go_sum_via_empty_hash() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write_gen_delta(root, &fixture_spec()).unwrap();
        let written = std::fs::read_to_string(root.join("Go.gen.lock")).unwrap();
        let back: GoGenDelta = serde_json::from_str(&written).unwrap();
        assert_eq!(back.go_sum_sha256, sha256_hex(b""));
    }
}
