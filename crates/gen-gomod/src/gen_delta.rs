//! `Go.gen.lock` — the slim resolver-delta for the gomod ecosystem.
//!
//! Mirrors gen-cargo's `Cargo.gen.lock` shape (a `GenDeltaArtifact`
//! tied to the source by a content hash), but carries what Go's build
//! actually needs incrementally: the **per-node `source_hash`** (the
//! incremental cache keys — a changed hash ⇒ rebuild that node + its
//! dependents) and the **`go_sum_sha256`** freshness tie the substrate
//! `go/lockfile-delta.nix` D2 gate already checks (empty-string hash
//! `e3b0c4…` when dep-free).
//!
//! Deliberately slim: it restates neither the import graph nor the file
//! lists (those live in `Go.build-spec.json`). Unlike Cargo — where the
//! lockfile reconstructs the dep closure in pure Nix — Go's package
//! graph is NOT reconstructable from `go.sum` (only `go list` resolves
//! build constraints + vendor rewrites), so the graph stays in the full
//! build-spec; this delta is purely the freshness + cache-key surface.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::build_spec::{BuildSpec, PackageKind};

/// Schema version of the slim delta artifact (distinct from the full
/// spec's `SCHEMA_VERSION`).
pub const DELTA_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Error)]
pub enum GenDeltaError {
    #[error("gen-delta: BuildSpec has no go_sum_sha256 — the D2 freshness tie is mandatory")]
    NoGoSumSha,
    #[error("gen-delta: refusing to emit a delta with zero source-hashed nodes")]
    EmptyNodes,
    #[error("gen-delta: serialize Go.gen.lock: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("gen-delta: write {path}: {source}")]
    Write {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

/// `Go.gen.lock`. Keyed maps are `BTreeMap` ⇒ canonical (sorted) key
/// order by construction, so the file is byte-identical across build
/// hosts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoGenDelta {
    pub schema_version: u32,
    /// D2 freshness tie — lowercase hex SHA-256 of `go.sum`.
    pub go_sum_sha256: String,
    /// Per-node incremental cache keys: node key → BLAKE3 `source_hash`.
    /// Std nodes (content-addressed by the std-tree, no per-file hash)
    /// are omitted.
    pub source_hashes: BTreeMap<String, String>,
}

impl GoGenDelta {
    /// Distill from a full v2 build-spec. Errors rather than emit a
    /// degenerate delta (missing tie / no hashed nodes).
    pub fn distill(spec: &BuildSpec) -> Result<Self, GenDeltaError> {
        let go_sum_sha256 = spec.go_sum_sha256.clone().ok_or(GenDeltaError::NoGoSumSha)?;
        let source_hashes: BTreeMap<String, String> = spec
            .packages
            .iter()
            .filter(|(_, p)| p.kind != PackageKind::Std && !p.source_hash.is_empty())
            .map(|(k, p)| (k.clone(), p.source_hash.clone()))
            .collect();
        if source_hashes.is_empty() {
            return Err(GenDeltaError::EmptyNodes);
        }
        Ok(GoGenDelta { schema_version: DELTA_SCHEMA_VERSION, go_sum_sha256, source_hashes })
    }

    pub fn to_json(&self) -> Result<String, GenDeltaError> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    pub const FILENAME: &'static str = "Go.gen.lock";
}

/// Distill + write `Go.gen.lock` next to the module's `go.mod`.
///
/// RETIRED: requires an `ActiveDelta` witness, which `DeltaPolicy::activate`
/// only mints when the declared mode is `Active`. The declaration ships as
/// retired, so this is unreachable without changing `specs/go-delta.lisp`.
/// Kept compiled and tested per MODULARIZE, DON'T DELETE.
pub(crate) fn write_gen_delta(
    _witness: &crate::delta_mode::ActiveDelta,
    root: &Path,
    spec: &BuildSpec,
) -> Result<(), GenDeltaError> {
    let delta = GoGenDelta::distill(spec)?;
    let path = root.join(GoGenDelta::FILENAME);
    std::fs::write(&path, delta.to_json()? + "\n").map_err(|source| GenDeltaError::Write {
        path: path.display().to_string(),
        source,
    })
}

#[cfg(test)]
mod golden {
    //! The BOTH-SIDES BINDING for `Go.gen.lock`.
    //!
    //! The GEN TYPED-SPEC CONTRACT requires every conditional emission rule to
    //! be a typed invariant asserted on both sides. Its absence is not
    //! hypothetical here: producer and consumer had already drifted to
    //! disagreeing on 12 of 13 fields, and the consumer defaulted the
    //! difference away with bare `or`.
    //!
    //! This pins the exact BYTES the producer emits. Its twin lives at
    //! `substrate/lib/build/go/tests/fixtures/gen-lock-current-gen/Go.gen.lock`
    //! and holds byte-identical content: substrate's suite asserts those bytes
    //! are REFUSED by the closed reader (they carry no `per_package`); this one
    //! asserts they are what the producer emits. One artifact, two claims,
    //! neither able to move alone.
    //!
    //! WHY BYTES, NOT THE STRUCT. `to_json` is `to_string_pretty`, so key order
    //! and indentation are part of the observable artifact. A struct-level
    //! assertion passes while the file on disk changes shape — the exact class
    //! of gap this milestone closes. Key order here is FIELD DECLARATION order
    //! (serde's struct behaviour), which is NOT serde_json's `json!` macro
    //! behaviour (alphabetical) — a first cut of this test used `json!` and
    //! disagreed with the real serializer, which is why it asserts on
    //! `GoGenDelta` itself.
    //!
    //! WHY IT LIVES IN THE CRATE. `gen_delta` is `pub(crate)` since the
    //! producer's retirement, so an integration test cannot construct
    //! `GoGenDelta`. That is the retirement working as intended; the golden
    //! moved inward rather than the module being re-opened for a test.
    //!
    //! TIER: only-mitigated (C2 — cross-repository observation). The two files
    //! cannot be diffed by one test runner; each side pins the same bytes and
    //! the invariant is enforced by both suites failing on a shape change.
    use super::*;

    const GOLDEN: &str = include_str!("../tests/fixtures/go-gen-lock-v1.json");

    fn golden_delta() -> GoGenDelta {
        let mut source_hashes = BTreeMap::new();
        source_hashes.insert("example.com/x/a#linux-amd64".to_string(), "aaaa".to_string());
        GoGenDelta {
            schema_version: 1,
            go_sum_sha256:
                "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".to_string(),
            source_hashes,
        }
    }

    #[test]
    fn golden_is_exactly_what_the_producer_emits() {
        assert_eq!(
            golden_delta().to_json().expect("serialize").trim_end(),
            GOLDEN.trim_end(),
            "the golden no longer matches what the producer emits — regenerate \
             BOTH this fixture and substrate's gen-lock-current-gen/Go.gen.lock, \
             or revert the shape change"
        );
    }

    #[test]
    fn golden_carries_no_per_package() {
        // The single fact substrate's reader keys on. If the shape ever gains
        // `per_package`, substrate's "current gen shape is refused" case becomes
        // wrong and must be updated in the same change.
        let v: serde_json::Value = serde_json::from_str(GOLDEN).expect("golden parses");
        assert!(
            v.get("per_package").is_none(),
            "golden gained `per_package`: substrate's refusal fixture is now stale"
        );
    }

    #[test]
    fn golden_top_level_keys_are_the_declared_three() {
        let v: serde_json::Value = serde_json::from_str(GOLDEN).expect("golden parses");
        let mut keys: Vec<&str> =
            v.as_object().expect("object").keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec!["go_sum_sha256", "schema_version", "source_hashes"],
            "the wire shape changed; substrate's delta-schema.nix declares these \
             three plus per_package and must be updated in lockstep"
        );
    }

    #[test]
    fn the_producer_that_would_write_this_is_retired() {
        // Binds the golden to the retirement: these bytes describe an artifact
        // nothing currently emits. If the policy is activated, this is the
        // reminder that substrate's reader must accept the shape first.
        assert!(crate::delta_mode::DeltaPolicy::RETIRED.is_retired());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_spec::{
        BuildTree, DepMode, EmbedSpec, GoPackageArgs, ModuleSpec, PackageSource, PackageSpec,
        Renderer, SCHEMA_VERSION,
    };

    fn node(kind: PackageKind, hash: &str) -> PackageSpec {
        PackageSpec {
            import_path: "example.com/x/a".into(),
            kind,
            source: if kind == PackageKind::Std {
                PackageSource::Std
            } else {
                PackageSource::Vendored { relative_path: "a".into() }
            },
            tree: BuildTree::Target,
            go_files: vec!["a.go".into()],
            build_tags: vec![],
            embed: EmbedSpec::default(),
            imports: vec![],
            import_map: BTreeMap::new(),
            module: None,
            source_hash: hash.into(),
            args: GoPackageArgs::default(),
            quirks: vec![],
        }
    }

    fn spec_with(go_sum: Option<&str>) -> BuildSpec {
        let mut packages = BTreeMap::new();
        packages.insert("example.com/x/a#linux-amd64".to_string(), node(PackageKind::Module, "aaaa"));
        packages.insert("std/fmt#linux-amd64".to_string(), node(PackageKind::Std, ""));
        BuildSpec {
            version: SCHEMA_VERSION,
            renderer: Renderer::Incremental,
            module: ModuleSpec {
                module_path: "example.com/x".into(),
                go_version: "1.25".into(),
                toolchain: None,
                has_external_deps: false,
                dep_mode: DepMode::Vendored,
                vendor_hash: None,
            },
            packages,
            root_package: "example.com/x/a#linux-amd64".into(),
            workspace_members: vec![],
            target_resolves: None,
            go_sum_sha256: go_sum.map(str::to_string),
        }
    }

    #[test]
    fn distill_carries_hashes_and_tie_omits_std() {
        let d = GoGenDelta::distill(&spec_with(Some("deadbeef"))).unwrap();
        assert_eq!(d.go_sum_sha256, "deadbeef");
        assert_eq!(d.source_hashes.len(), 1, "std node omitted, module node kept");
        assert!(d.source_hashes.contains_key("example.com/x/a#linux-amd64"));
    }

    #[test]
    fn missing_go_sum_tie_is_refused() {
        assert!(matches!(
            GoGenDelta::distill(&spec_with(None)),
            Err(GenDeltaError::NoGoSumSha)
        ));
    }

    #[test]
    fn roundtrip_is_lossless() {
        let d = GoGenDelta::distill(&spec_with(Some("deadbeef"))).unwrap();
        let json = d.to_json().unwrap();
        let back: GoGenDelta = serde_json::from_str(&json).unwrap();
        assert_eq!(d, back);
        assert_eq!(json, back.to_json().unwrap());
    }
}
