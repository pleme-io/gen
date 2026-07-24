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

/// `Cargo.gen.lock` — the slim resolver delta. Struct field order is the JSON
/// layout; the keyed maps are `BTreeMap` so key order is canonical (sorted) by
/// construction — cross-platform byte-stable, independent of the build host's
/// cargo resolve-traversal order.
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
    /// Serialize to pretty JSON. Cross-platform byte-stable: the keyed maps
    /// are `BTreeMap` (canonical key order by construction) and the spec's
    /// resolve-ordered Vecs are pre-sorted by `BuildSpec::canonicalize()`
    /// before distillation, so identical content yields identical bytes on
    /// any build host.
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

// ── OFFLINE delta-freshness check — the D2 tie, verified from committed
//    artifacts alone (no cargo, no network, no `~/.cargo`) ───────────────────

/// Minimal decode of `Cargo.gen.lock` — ONLY the freshness tie. Decoupled
/// from the full [`GenDelta`] (same discipline as `build_spec::SpecHeader`):
/// a delta whose resolver subtree has evolved (a new `PerCrateScalars` field,
/// a bumped `schema_version`) still yields a usable freshness reading, because
/// the D2 tie is all `gen confirm` consults.
#[derive(Deserialize)]
struct DeltaShaHeader {
    #[serde(default)]
    cargo_lock_sha256: Option<String>,
}

/// The offline delta-freshness verdict. Computed by reading ONLY `Cargo.lock`
/// + the committed `Cargo.gen.lock` at the workspace root — no `cargo
/// metadata`, no network, no `~/.cargo`. Serialized as a discriminated union
/// (`status` tag) an operator or CI gate can branch on.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum DeltaFreshness {
    /// `sha256(Cargo.lock)` matches the delta's recorded `cargo_lock_sha256`.
    Fresh { cargo_lock_sha256: String },
    /// The delta's recorded sha differs from the current `Cargo.lock` — the
    /// lock changed without regenerating `Cargo.gen.lock` (`gen build`).
    Stale { expected: String, actual: String },
    /// No `Cargo.gen.lock` at the workspace root (never generated, or lost).
    MissingDelta { actual: String },
    /// `Cargo.gen.lock` exists but carries no `cargo_lock_sha256` tie
    /// (corrupt, or a pre-D2 artifact) — cannot prove freshness, so stale.
    UntiedDelta { actual: String },
    /// No `Cargo.lock` at the workspace root — nothing to tie against.
    MissingLock,
}

impl DeltaFreshness {
    /// True unless the committed delta is PROVEN fresh against its lock.
    /// Drives the non-zero exit code of `gen confirm` — a missing / untied /
    /// mismatched delta all gate the build, never silently pass.
    #[must_use]
    pub fn is_stale(&self) -> bool {
        !matches!(self, DeltaFreshness::Fresh { .. })
    }

    /// One-line operator summary.
    #[must_use]
    pub fn summary(&self) -> String {
        match self {
            DeltaFreshness::Fresh { .. } => {
                "fresh: Cargo.gen.lock matches Cargo.lock".to_string()
            }
            DeltaFreshness::Stale { expected, actual } => format!(
                "STALE: Cargo.lock changed without `gen build` — \
                 Cargo.gen.lock records {expected}, Cargo.lock is {actual}"
            ),
            DeltaFreshness::MissingDelta { .. } => {
                "MISSING: no Cargo.gen.lock — run `gen build`".to_string()
            }
            DeltaFreshness::UntiedDelta { .. } => {
                "UNTIED: Cargo.gen.lock has no cargo_lock_sha256 — run `gen build`".to_string()
            }
            DeltaFreshness::MissingLock => "MISSING: no Cargo.lock".to_string(),
        }
    }
}

/// Pure, OFFLINE delta-freshness check — the D2 freshness tie verified from
/// the committed artifacts alone. Reads ONLY `Cargo.lock` and `Cargo.gen.lock`
/// at `root`: recomputes `sha256(Cargo.lock)` (via `build_spec::hash_cargo_lock`
/// — the SAME digest the producer wrote, identical to `builtins.hashFile
/// "sha256"`) and compares it to the delta's recorded `cargo_lock_sha256`.
///
/// No `cargo metadata`, no network, no `~/.cargo` registry — safe inside a Nix
/// `runCommand` / `nix flake check` sandbox. This is the offline-safe
/// DESTINATION of the `gen-confirm` substrate check: `gen check` proves the
/// workspace PARSES; `gen confirm` proves the committed delta is FRESH against
/// its lock (a `cargo update` without a `gen build` is caught, offline).
#[must_use]
pub fn confirm_freshness(root: &Path) -> DeltaFreshness {
    let actual = match crate::build_spec::hash_cargo_lock(root) {
        Some(h) => h,
        None => return DeltaFreshness::MissingLock,
    };
    let delta_path = root.join(GenDelta::FILENAME);
    let text = match std::fs::read_to_string(&delta_path) {
        Ok(t) => t,
        Err(_) => return DeltaFreshness::MissingDelta { actual },
    };
    let header: DeltaShaHeader = match serde_json::from_str(&text) {
        Ok(h) => h,
        // A `Cargo.gen.lock` we cannot even read the tie out of cannot prove
        // freshness — refuse it rather than round up to fresh.
        Err(_) => return DeltaFreshness::UntiedDelta { actual },
    };
    match header.cargo_lock_sha256 {
        Some(expected) if expected == actual => {
            DeltaFreshness::Fresh { cargo_lock_sha256: actual }
        }
        Some(expected) => DeltaFreshness::Stale { expected, actual },
        None => DeltaFreshness::UntiedDelta { actual },
    }
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

    // ── confirm_freshness: pure OFFLINE D2-tie verification ──────────────

    fn confirm_tmpdir() -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static C: AtomicU64 = AtomicU64::new(0);
        let n = C.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!(
            "gen-confirm-freshness-{}-{}",
            std::process::id(),
            n
        ));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    /// The exact digest `confirm_freshness` will recompute for a given
    /// `Cargo.lock` body — mirrors `build_spec::hash_cargo_lock`.
    fn sha256_hex(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(bytes);
        format!("{:x}", h.finalize())
    }

    fn write_gen_lock(dir: &Path, sha: &str) {
        // A minimal, schema-valid-enough `Cargo.gen.lock` for the tie check.
        // (The full struct requires target_resolves; the offline confirm only
        // consults `cargo_lock_sha256`, so a minimal doc is the honest fixture.)
        let body = format!(r#"{{ "schema_version": 1, "cargo_lock_sha256": "{sha}" }}"#);
        std::fs::write(dir.join("Cargo.gen.lock"), body).unwrap();
    }

    #[test]
    fn confirm_fresh_when_sha_matches() {
        let dir = confirm_tmpdir();
        let lock = b"# fresh lock\n[[package]]\nname = \"x\"\n";
        std::fs::write(dir.join("Cargo.lock"), lock).unwrap();
        write_gen_lock(&dir, &sha256_hex(lock));
        let v = confirm_freshness(&dir);
        assert_eq!(v, DeltaFreshness::Fresh { cargo_lock_sha256: sha256_hex(lock) });
        assert!(!v.is_stale());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn confirm_stale_when_lock_edited_without_regen() {
        let dir = confirm_tmpdir();
        let old_lock = b"# old lock\n";
        // Cargo.gen.lock records the OLD lock's sha ...
        write_gen_lock(&dir, &sha256_hex(old_lock));
        // ... but Cargo.lock has since changed (a `cargo update` with no
        // `gen build`): the delta is now stale.
        let new_lock = b"# new lock after cargo update\n";
        std::fs::write(dir.join("Cargo.lock"), new_lock).unwrap();
        let v = confirm_freshness(&dir);
        assert_eq!(
            v,
            DeltaFreshness::Stale {
                expected: sha256_hex(old_lock),
                actual: sha256_hex(new_lock),
            }
        );
        assert!(v.is_stale());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn confirm_missing_delta_is_stale() {
        let dir = confirm_tmpdir();
        std::fs::write(dir.join("Cargo.lock"), b"# lock\n").unwrap();
        // no Cargo.gen.lock
        let v = confirm_freshness(&dir);
        assert!(matches!(v, DeltaFreshness::MissingDelta { .. }));
        assert!(v.is_stale());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn confirm_untied_delta_is_stale() {
        let dir = confirm_tmpdir();
        std::fs::write(dir.join("Cargo.lock"), b"# lock\n").unwrap();
        // A Cargo.gen.lock with no cargo_lock_sha256 tie.
        std::fs::write(dir.join("Cargo.gen.lock"), r#"{ "schema_version": 1 }"#).unwrap();
        let v = confirm_freshness(&dir);
        assert!(matches!(v, DeltaFreshness::UntiedDelta { .. }));
        assert!(v.is_stale());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn confirm_missing_lock() {
        let dir = confirm_tmpdir();
        // no Cargo.lock at all
        write_gen_lock(&dir, "deadbeef");
        assert_eq!(confirm_freshness(&dir), DeltaFreshness::MissingLock);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
