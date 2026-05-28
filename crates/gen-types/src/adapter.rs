//! Canonical Adapter trait — the cross-ecosystem SDLC interface.
//!
//! Every ecosystem adapter (`gen-cargo`, `gen-npm`, `gen-bundler`,
//! `gen-pip`, `gen-gomod`, `gen-helm`, ...) implements one trait;
//! substrate's build wrappers and operator-facing CLIs both code to
//! this trait. Adding a new ecosystem gives every downstream consumer
//! + every renderer the same verb surface for free.
//!
//! ## Verbs
//!
//! - `lock`    — author's manifest → freshly resolved lockfile
//! - `build`   — lockfile + manifest → typed build-spec (hermetic;
//!               substrate calls this via IFD inside the nix sandbox)
//! - `plan`    — diff "what would change if I bump <input>"
//! - `confirm` — verify spec invariants (hashes valid, no orphans,
//!               features unify, source/lockfile match)
//! - `diff`    — human-readable diff between two states
//! - `sbom`    — emit ecosystem-flavored SBOM (CycloneDX / SPDX)
//!
//! ## Hermetic contract
//!
//! The `build` verb MUST run without network access — it's called
//! from inside a nix sandbox via IFD by substrate's `mkBuildSpec`.
//! Implementations that read from `Cargo.lock` / `package-lock.json`
//! / `Gemfile.lock` directly (without invoking the ecosystem's
//! native resolver) satisfy this. Implementations that need to
//! resolve transitively can do so during `lock`, never `build`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Result alias for adapter operations.
pub type AdapterResult<T> = Result<T, AdapterError>;

/// The canonical SDLC trait. Each ecosystem adapter implements one.
pub trait Adapter: Send + Sync {
    /// Short ecosystem identifier (e.g. `"cargo"`, `"npm"`, `"bundler"`).
    /// Used for routing operator commands + matching manifests.
    fn name(&self) -> &'static str;

    /// Manifest filenames this adapter recognizes — the first one
    /// found in the workspace dispatches to this adapter. Examples:
    /// - cargo: `["Cargo.toml"]`
    /// - npm:   `["package.json"]`
    /// - bundler: `["Gemfile"]`
    fn manifest_files(&self) -> &'static [&'static str];

    /// `gen lock <path>` — resolve the manifest to a fresh lockfile.
    /// May read the network (this is the resolver invocation).
    /// Idempotent: re-running with no manifest changes is a no-op.
    fn lock(&self, ctx: &AdapterCtx) -> AdapterResult<LockOutcome>;

    /// `gen build <path>` — manifest + lockfile → typed build-spec.
    ///
    /// MUST be hermetic. Reads from disk only. Substrate's
    /// `mkBuildSpec` wrapper invokes this from inside the nix
    /// sandbox; any network call breaks IFD.
    fn build(&self, ctx: &AdapterCtx) -> AdapterResult<BuildSpec>;

    /// `gen plan <path>` — given an intent (bump X, enable feature Y),
    /// produce the diff of what would change. Read-only. Used by
    /// pre-bump confirmation flows.
    fn plan(&self, ctx: &AdapterCtx, intent: &PlanIntent) -> AdapterResult<Plan>;

    /// `gen confirm <path>` — verify spec invariants:
    /// - every lockfile entry has a manifest declaration
    /// - every transitive dep has a hash
    /// - features resolve consistently across the workspace
    /// - source URLs are valid (no `?branch=` query leaks, etc.)
    /// Returns a typed report rather than panicking.
    fn confirm(&self, ctx: &AdapterCtx) -> AdapterResult<ConfirmReport>;

    /// `gen diff <path> <ref>` — diff current state vs a reference
    /// (file path, git rev, or "previous").
    fn diff(&self, ctx: &AdapterCtx, against: &DiffRef) -> AdapterResult<DiffReport>;

    /// `gen sbom <path>` — software bill of materials in the given
    /// format. Pure function of the build-spec.
    fn sbom(&self, ctx: &AdapterCtx, format: SbomFormat) -> AdapterResult<Sbom>;

    /// `gen quirks list` — typed registry of upstream third-party
    /// package quirks the adapter knows about. Each adapter's quirks
    /// shape is its own (CrateQuirk for Cargo, future NpmQuirk for
    /// npm, GemQuirk for Bundler) — the trait surface stays uniform
    /// via the opaque `serde_json::Value` envelope.
    ///
    /// Default `quirks_registry` returns empty — adapters with a
    /// registry override to expose it. Substrate consumers can
    /// introspect every ecosystem's quirks through a single call.
    ///
    /// The CANONICAL surface for adding a new quirk is the adapter's
    /// own typed registry; this trait method is the operator-facing
    /// + tooling-discovery surface.
    fn quirks_registry(&self) -> Vec<AdapterQuirkEntry> {
        Vec::new()
    }
}

/// One entry in an Adapter's quirks registry: a package name plus
/// the ecosystem-specific typed-quirk payloads. The `quirks` field is
/// `serde_json::Value` so each adapter can carry its own typed shape
/// without polluting the trait with a generic parameter — the JSON
/// envelope is what substrate's dispatch layer reads anyway.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AdapterQuirkEntry {
    /// Package name (`crate.name` for Cargo, `package.json` `name`
    /// for npm, gemspec `name` for Bundler).
    pub package: String,
    /// Quirk payloads, opaque to the trait. Each adapter shapes
    /// these as its own typed enum (gen-cargo: `CrateQuirk`); the
    /// JSON form is what substrate dispatches on.
    pub quirks: Vec<serde_json::Value>,
}

/// Context every adapter verb receives. Captures workspace root +
/// optional target filter. Extending this struct is non-breaking
/// because verbs take it by reference.
#[derive(Debug, Clone)]
pub struct AdapterCtx {
    /// Workspace root directory. The manifest_files entry lives at
    /// `${workspace_root}/${manifest_files()[i]}`.
    pub workspace_root: PathBuf,

    /// Optional target predicate (e.g. `aarch64-apple-darwin`). When
    /// `Some`, the adapter restricts the resolve graph to deps
    /// active for that target.
    pub target: Option<String>,
}

/// Outcome of a `lock` invocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockOutcome {
    /// Absolute path to the written lockfile.
    pub lockfile_path: PathBuf,
    /// True if the lockfile was created from scratch (no prior
    /// lockfile existed). False = update of existing lockfile.
    pub created: bool,
    /// Per-dependency bumps applied. Empty when manifest didn't
    /// change. Useful for changelog generation.
    pub bumped: Vec<DependencyBump>,
}

/// One dep that changed version during a lock invocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyBump {
    pub name: String,
    pub from: Option<String>,
    pub to: String,
}

/// Output of a `build` invocation. The JSON shape is
/// ecosystem-specific — Cargo.build-spec.json / package-lock.spec.json
/// / etc. — but every adapter wraps it in this typed envelope so
/// substrate can read uniformly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildSpec {
    /// Ecosystem name (mirrors `Adapter::name()`).
    pub ecosystem: String,
    /// Spec schema version. Bumped on breaking changes to the JSON
    /// shape; consumers can refuse-or-warn.
    pub schema_version: u32,
    /// Ecosystem-shaped JSON payload.
    pub data: serde_json::Value,
}

/// Operator-supplied intent for `gen plan`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlanIntent {
    /// "What if I bump this dep to latest" (one entry per dep).
    pub bumps: Vec<String>,
    /// "What if I enable these features" (root crate / workspace).
    pub enable_features: Vec<String>,
    /// "What if I disable these features"
    pub disable_features: Vec<String>,
}

/// Result of `plan`. The diff describes the resulting state minus
/// the current state; warnings flag anything risky (yanked, MSRV
/// bump, semver-major, license change, ...).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub diff: DiffReport,
    pub warnings: Vec<PlanWarning>,
}

/// One warning emitted during planning.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanWarning {
    pub severity: PlanWarningSeverity,
    pub message: String,
    pub dep: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanWarningSeverity {
    Info,
    Warn,
    Block,
}

/// Result of `confirm`. Each invariant is named so failures point
/// at the exact rule that broke; operators get actionable output
/// instead of a stack trace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfirmReport {
    pub invariants_held: Vec<String>,
    pub invariants_broken: Vec<InvariantBreak>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvariantBreak {
    /// Stable name of the invariant (e.g. `"hash-present"`).
    pub name: String,
    /// Human-readable explanation.
    pub message: String,
    /// Optional pointer to the offending dep / file.
    pub locus: Option<String>,
}

/// Reference for `diff` — what to compare current state against.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum DiffRef {
    /// Previous committed state (git HEAD or HEAD~1).
    Previous,
    /// Specific git revision.
    GitRev { rev: String },
    /// Spec file on disk (operator supplied).
    Path { path: PathBuf },
}

/// Diff output for `plan` / `diff`. Three buckets: added,
/// removed, version-changed. Consumers (PR comment renderer, CI
/// gate, ...) pick what to surface.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffReport {
    pub added: Vec<DepEdge>,
    pub removed: Vec<DepEdge>,
    pub changed: Vec<DepChange>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DepEdge {
    pub name: String,
    pub version: String,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DepChange {
    pub name: String,
    pub from_version: String,
    pub to_version: String,
    pub kind: String,
}

/// SBOM output format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SbomFormat {
    CycloneDx,
    Spdx,
}

/// Generated SBOM. Format-flavored JSON; consumers route by `format`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sbom {
    pub format: SbomFormat,
    pub data: serde_json::Value,
}

/// Adapter-level errors. Variants stay narrow so callers can
/// pattern-match on recoverable cases.
#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    #[error("manifest not found at {0}")]
    ManifestNotFound(PathBuf),

    #[error("lockfile not found at {0} — run `gen lock` first")]
    LockfileNotFound(PathBuf),

    #[error("hermetic constraint violated: {0}")]
    NonHermetic(String),

    #[error("invariant broken: {name}: {message}")]
    InvariantBroken { name: String, message: String },

    #[error("unsupported operation: {0}")]
    Unsupported(String),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("internal: {0}")]
    Internal(String),
}

/// Inventory entry registered by every adapter crate. gen-cli's
/// `quirks` / `adapters` / future cross-ecosystem verbs iterate
/// this distributed slice to discover every adapter at link time —
/// no hard-coded match arms, no per-ecosystem edits to gen-cli when
/// a new adapter lands.
///
/// Adapters register via `inventory::submit!`. See
/// `gen-cargo::adapter` for the canonical example.
pub struct AdapterRegistration {
    /// Constructs a fresh instance of the adapter. `&dyn Adapter`
    /// can be obtained by `(&*reg.make())`.
    pub make: fn() -> Box<dyn Adapter>,
    /// Adapter name (mirror of `Adapter::name`). Allows filtering
    /// without instantiating the adapter — used by
    /// `gen quirks --adapter <name>`.
    pub name: &'static str,
}

inventory::collect!(AdapterRegistration);

/// Iterate every adapter the binary was linked against. Returns
/// freshly-constructed instances so callers can hold them
/// independently. Per-call O(N) — N = number of linked adapters,
/// typically small (3-20).
#[must_use]
pub fn registered_adapters() -> Vec<Box<dyn Adapter>> {
    inventory::iter::<AdapterRegistration>()
        .map(|r| (r.make)())
        .collect()
}

/// Get one adapter by name, freshly constructed. Returns None if
/// no adapter registered under that name.
#[must_use]
pub fn adapter_by_name(name: &str) -> Option<Box<dyn Adapter>> {
    inventory::iter::<AdapterRegistration>()
        .find(|r| r.name == name)
        .map(|r| (r.make)())
}

/// Every registered adapter's name. Used for `gen adapters` listing
/// + introspection without constructing the adapters.
#[must_use]
pub fn registered_adapter_names() -> Vec<&'static str> {
    inventory::iter::<AdapterRegistration>()
        .map(|r| r.name)
        .collect()
}
