//! `gen-cargo` slim resolver-delta — `Cargo.gen.lock`.
//!
//! The committed artifact that supersedes the full `Cargo.build-spec.json`:
//! it carries ONLY the cargo-resolver facts that `Cargo.lock` cannot express
//! (per-target resolved features + dep edges, per-crate scalars, git NAR
//! sha256, module-trio), tied to the lock by `cargo_lock_sha256`. Everything
//! the lock already pins (name/version/source/checksum/dep-closure) is
//! reconstructed in pure Nix via `builtins.fromTOML` by substrate's
//! `lockfile-builder.nix` — never restated here.
//!
//! Contract: `gen/docs/CARGO-LOCK-DELTA-CONTRACT.md` (D1–D4). This module is
//! the PRODUCER; substrate is the CONSUMER. Additive — the full build-spec
//! emit is untouched; `write_gen_delta` runs alongside it.

use std::path::Path;

// BTreeMap (not IndexMap): the delta's keyed maps are canonical (sorted-by-key)
// by construction, so `Cargo.gen.lock` is byte-identical across build platforms.
// They are cloned/collected from the already-canonical BuildSpec.
use std::collections::BTreeMap;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::build_spec::{
    BuildSpec, CompactTargetResolves, CrateBinSpec, CrateSource, LibTargetSpec, ModuleTrioSpec,
};
use crate::quirks::CrateQuirk;

/// Schema version of the slim delta artifact. Distinct from
/// `build_spec::SCHEMA_VERSION` (the full spec's version) — the consumer
/// gates decode on this.
pub const DELTA_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Error)]
pub enum GenDeltaError {
    #[error(
        "gen-delta: BuildSpec has no target_resolves (single-target spec) — cannot \
         emit a fleet-correct delta; emit a multi-target spec first"
    )]
    NoTargetResolves,
    #[error(
        "gen-delta: BuildSpec has no cargo_lock_sha256 — the D2 freshness tie is \
         mandatory; re-run `gen build` on a schema-v7+ spec"
    )]
    NoLockSha,
    #[error("gen-delta: refusing to emit a delta with zero crates (D4)")]
    EmptyCrates,
    #[error("gen-delta: serialize Cargo.gen.lock: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("gen-delta: write {path}: {source}")]
    Write {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

/// A slim, committed resolver-delta: the minimal facts an ecosystem's
/// lockfile cannot express, kept in lockstep with that lock via a content
/// hash. The rust impl is the POC; npm/python/go get the same shape (see
/// the contract's "Generalization" section) — hence a trait, not a bare fn.
pub trait GenDeltaArtifact: Sized {
    /// The full in-memory spec this delta is distilled from.
    type FullSpec;
    /// Distillation error type (per-ecosystem).
    type Error: std::error::Error;
    /// Artifact schema version (gates consumer decode).
    const SCHEMA_VERSION: u32;
    /// The committed filename (e.g. `Cargo.gen.lock`).
    const FILENAME: &'static str;

    /// Distill the slim delta from the full spec. MUST drop every field the
    /// lockfile already expresses (D1) and MUST error rather than emit a
    /// degenerate delta (D4): a single-target spec lacks the per-target
    /// resolver facts the delta exists to carry.
    fn distill(full: &Self::FullSpec) -> Result<Self, Self::Error>;

    /// The freshness tie — equals `builtins.hashFile "sha256"` of the lock
    /// at consume time (D2). Lowercase hex SHA-256.
    fn lock_sha256(&self) -> &str;
}

/// The resolver-only, target-invariant scalars for one crate. Mirrors the
/// subset of `CrateSpec` that cannot be derived from `Cargo.lock`. Every
/// field here is allow-listed by the D1 test; adding a field means updating
/// that allow-list deliberately.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PerCrateScalars {
    pub edition: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub proc_macro: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_script: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub links: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lib_target: Option<LibTargetSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub binaries: Vec<CrateBinSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quirks: Vec<CrateQuirk>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// Per-member metadata the lock can't express. `default_bin`/`repo` are
/// intentionally absent — both are derivable in Nix (default-bin rule;
/// `[package].repository`), so committing them would violate D1.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MemberDelta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module_trio: Option<ModuleTrioSpec>,
}

/// `Cargo.gen.lock` — the slim resolver delta. Field order is the JSON
/// layout; `IndexMap` keeps emission deterministic.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GenDelta {
    pub schema_version: u32,
    /// D2 freshness tie. Lowercase hex SHA-256 of `Cargo.lock`.
    pub cargo_lock_sha256: String,
    /// The per-target resolver edges + features — carried VERBATIM from the
    /// full spec (already compacted as base // overrides[triple]). This is
    /// the bulk of the delta and the whole reason it can't be empty.
    pub target_resolves: CompactTargetResolves,
    /// Target-invariant per-crate scalars, keyed by `<name>-<version>`.
    pub per_crate: BTreeMap<String, PerCrateScalars>,
    /// Git-source NAR sha256 (SRI `sha256-<base64>`), keyed by crate key.
    /// The lock carries the rev; this fixed-output hash is gen's prefetch.
    #[serde(default)]
    pub git_nar_sha256: BTreeMap<String, String>,
    /// Per-member module-trio specs (only members that authored
    /// `[package.metadata.pleme]`).
    #[serde(default)]
    pub flake_metadata: BTreeMap<String, MemberDelta>,
}

impl GenDeltaArtifact for GenDelta {
    type FullSpec = BuildSpec;
    type Error = GenDeltaError;
    const SCHEMA_VERSION: u32 = DELTA_SCHEMA_VERSION;
    const FILENAME: &'static str = "Cargo.gen.lock";

    fn distill(spec: &BuildSpec) -> Result<Self, GenDeltaError> {
        let target_resolves = spec
            .target_resolves
            .clone()
            .ok_or(GenDeltaError::NoTargetResolves)?;
        // A pre-v10 spec deserializes its old per-triple target_resolves into
        // an EMPTY CompactTargetResolves — a degenerate delta carrying no
        // resolver edges. Refuse it: the repo's spec must be regenerated to
        // the v10 compact shape (`gen build`) before a delta is meaningful.
        if target_resolves.base.is_empty() && target_resolves.targets.is_empty() {
            return Err(GenDeltaError::NoTargetResolves);
        }
        let cargo_lock_sha256 = spec
            .cargo_lock_sha256
            .clone()
            .ok_or(GenDeltaError::NoLockSha)?;

        let per_crate: BTreeMap<String, PerCrateScalars> = spec
            .crates
            .iter()
            .map(|(key, c)| {
                (
                    key.clone(),
                    PerCrateScalars {
                        edition: c.edition.clone(),
                        proc_macro: c.proc_macro,
                        build_script: c.build_script.clone(),
                        links: c.links.clone(),
                        lib_target: c.lib_target.clone(),
                        binaries: c.binaries.clone(),
                        quirks: c.quirks.clone(),
                    },
                )
            })
            .collect();

        if per_crate.is_empty() {
            return Err(GenDeltaError::EmptyCrates);
        }

        // Git NAR sha256: the lock has the rev, never this fixed-output hash.
        let git_nar_sha256: BTreeMap<String, String> = spec
            .crates
            .iter()
            .filter_map(|(key, c)| match &c.source {
                CrateSource::Git {
                    sha256: Some(h), ..
                } => Some((key.clone(), h.clone())),
                _ => None,
            })
            .collect();

        let flake_metadata: BTreeMap<String, MemberDelta> = spec
            .flake_metadata
            .iter()
            .filter_map(|(name, m)| {
                m.module_trio.clone().map(|t| {
                    (
                        name.clone(),
                        MemberDelta {
                            module_trio: Some(t),
                        },
                    )
                })
            })
            .collect();

        Ok(GenDelta {
            schema_version: DELTA_SCHEMA_VERSION,
            cargo_lock_sha256,
            target_resolves,
            per_crate,
            git_nar_sha256,
            flake_metadata,
        })
    }

    fn lock_sha256(&self) -> &str {
        &self.cargo_lock_sha256
    }
}

impl GenDelta {
    /// Serialize to pretty JSON (deterministic via `IndexMap` ordering).
    pub fn to_json(&self) -> Result<String, GenDeltaError> {
        Ok(serde_json::to_string_pretty(self)?)
    }
}

/// Distill + write `Cargo.gen.lock` next to the workspace's `Cargo.lock`.
/// Additive — call after the full build-spec write. Propagates errors: a
/// failed delta emit MUST fail `gen build` (never silently skipped).
pub fn write_gen_delta(root: &Path, spec: &BuildSpec) -> Result<(), GenDeltaError> {
    let delta = GenDelta::distill(spec)?;
    let path = root.join(GenDelta::FILENAME);
    std::fs::write(&path, delta.to_json()? + "\n").map_err(|source| GenDeltaError::Write {
        path: path.display().to_string(),
        source,
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    // Real v10 fixture: a committed sample multi-target build-spec
    // (testdata/). Self-contained — does NOT depend on gen tracking its
    // own Cargo.build-spec.json, which is retired under the delta-only
    // doctrine (gitignored, reconstructed from Cargo.gen.lock).
    fn fixture() -> BuildSpec {
        let raw = include_str!("testdata/v10-build-spec.json");
        serde_json::from_str(raw).expect("fixture v10-build-spec.json parses")
    }

    fn delta() -> GenDelta {
        GenDelta::distill(&fixture()).expect("distill succeeds on a v10 multi-target spec")
    }

    // ── D4: the delta is non-empty by necessity ──────────────────────
    #[test]
    fn d4_delta_is_non_empty() {
        let d = delta();
        assert!(!d.per_crate.is_empty(), "per_crate must be populated");
        assert!(
            !d.target_resolves.base.is_empty() || !d.target_resolves.targets.is_empty(),
            "target_resolves must carry the resolver edges"
        );
    }

    // ── D2: freshness tie is a lowercase hex SHA-256 ─────────────────
    #[test]
    fn d2_freshness_tie_is_lowercase_hex_sha256() {
        let d = delta();
        let s = d.lock_sha256();
        assert_eq!(s.len(), 64, "sha256 hex is 64 chars");
        assert!(
            s.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "must be lowercase hex to match builtins.hashFile \"sha256\""
        );
    }

    // ── D1: no restatement of lock-owned fields in the envelope ──────
    // The envelope = the delta minus the `target_resolves` subtree (where
    // `name`/`version`/`features`/edge `package_key` legitimately appear).
    #[test]
    fn d1_envelope_carries_no_lock_owned_fields() {
        let d = delta();
        let mut v: Value = serde_json::to_value(&d).unwrap();
        v.as_object_mut().unwrap().remove("target_resolves");

        const FORBIDDEN: &[&str] = &[
            "source",
            "name_with_ext",
            "relative_path",
            "rev",
            "checksum",
            "url",
            "root_crate",
            "workspace_members",
            "build_rust_crate_args",
            "crate_renames",
            "default_bin",
            "repo",
            "dependencies",
            "runtime_dependencies",
            "build_dependencies",
        ];
        let mut seen: Vec<String> = Vec::new();
        collect_keys(&v, &mut seen);
        for f in FORBIDDEN {
            assert!(
                !seen.iter().any(|k| k == f),
                "D1 violation: lock-owned key `{f}` leaked into the delta envelope"
            );
        }
    }

    // ── D1 corollary: per_crate entries carry only allow-listed scalars
    #[test]
    fn d1_per_crate_allowlist_only() {
        let d = delta();
        let v: Value = serde_json::to_value(&d).unwrap();
        const ALLOW: &[&str] = &[
            "edition",
            "proc_macro",
            "build_script",
            "links",
            "lib_target",
            "binaries",
            "quirks",
        ];
        for (key, entry) in v["per_crate"].as_object().unwrap() {
            for field in entry.as_object().unwrap().keys() {
                assert!(
                    ALLOW.contains(&field.as_str()),
                    "per_crate[{key}] has non-allow-listed field `{field}` (D1)"
                );
            }
        }
    }

    // ── Round-trip: distill → serialize → deserialize is lossless ────
    #[test]
    fn roundtrip_is_lossless() {
        let d = delta();
        let json = d.to_json().unwrap();
        let back: GenDelta = serde_json::from_str(&json).unwrap();
        assert_eq!(
            json,
            back.to_json().unwrap(),
            "Cargo.gen.lock must round-trip byte-stably"
        );
    }

    // ── git NAR sha256, when present, is SRI (never bare hex) ─────────
    #[test]
    fn git_nar_sha256_is_sri() {
        let d = delta();
        for (key, h) in &d.git_nar_sha256 {
            assert!(
                h.starts_with("sha256-"),
                "git_nar_sha256[{key}] must be SRI (sha256-<base64>), got `{h}`"
            );
        }
    }

    // Emit gen's own `Cargo.gen.lock` from the committed build-spec fixture
    // (no network). Run: `cargo test -p gen-cargo emit_fixture_gen_lock -- --ignored`.
    // Used by the substrate lockfile-delta eval-equivalence oracle.
    #[test]
    #[ignore]
    fn emit_gen_lock_for() {
        // GEN_DELTA_SPEC=<build-spec.json> GEN_DELTA_OUT=<Cargo.gen.lock>
        // (default: gen's own). Distills the slim delta from any build-spec
        // for the substrate lockfile-delta equivalence oracle.
        let spec_path = std::env::var("GEN_DELTA_SPEC")
            .unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../../Cargo.build-spec.json").into());
        let out = std::env::var("GEN_DELTA_OUT")
            .unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../../Cargo.gen.lock").into());
        let spec: BuildSpec =
            serde_json::from_str(&std::fs::read_to_string(&spec_path).unwrap()).unwrap();
        let d = GenDelta::distill(&spec).unwrap();
        std::fs::write(&out, d.to_json().unwrap() + "\n").unwrap();
        eprintln!("wrote {out} from {spec_path}");
    }

    fn collect_keys(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::Object(m) => {
                for (k, child) in m {
                    out.push(k.clone());
                    collect_keys(child, out);
                }
            }
            Value::Array(a) => a.iter().for_each(|c| collect_keys(c, out)),
            _ => {}
        }
    }
}
