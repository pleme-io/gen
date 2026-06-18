//! `Cargo.build-spec.json` — the canonical complete typed build manifest.
//!
//! Composes Cargo.toml + Cargo.lock + cargo metadata into one typed
//! JSON that the substrate Nix builder consumes directly with
//! `builtins.fromJSON`. Nix becomes a pure orchestrator — no parsing,
//! no string splitting, no semantic resolution.
//!
//! This is the architectural split: Rust owns ALL semantics
//! (parsing, dep resolution, feature activation, src URL synthesis,
//! sha256 plumbing, rename handling, optional gating, target
//! conditions); Nix owns dispatch (one JSON read, per-crate
//! buildRustCrate calls, attrset assembly).

use std::path::Path;

use cargo_metadata::MetadataCommand;
use indexmap::IndexMap;
// Output-bearing keyed-by-string maps use BTreeMap so their JSON key order is
// canonical (lexicographic) BY CONSTRUCTION — the serialized spec is then
// byte-identical regardless of cargo's resolve-traversal order, which is
// platform-dependent (linux CI vs darwin). IndexMap stays for working/
// intermediate maps whose order is FLEET_TARGETS-deterministic or never
// serialized. (Determinism canonicalization — GEN TYPED-SPEC CONTRACT.)
use std::collections::BTreeMap;
use serde::{Deserialize, Serialize};

use crate::error::{CargoError, Result};

/// Schema version — bump on breaking changes.
/// v2: + `flake_metadata`, `root_crate` is non-optional, git URLs
///     normalized (no `?branch=` suffix).
/// v3: + per-crate `build_rust_crate_args` (pre-shaped buildRustCrate
///     kwargs); + `links` + universal `preBuild`. Substrate's
///     lockfile-builder asserts on this version — older specs MUST
///     be regenerated via `gen build .` (no silent fallback).
/// v9: drop redundant `dependencies` union from JSON — Nix consumes only
///     runtime_dependencies + build_dependencies; the union is
///     reconstructable and was never read.
/// v10: compact `target_resolves` = `base` (cross-target-identical edges,
///     stored once) + per-target `overrides`; Nix decodes
///     `base // overrides[triple]`. Collapses the ~41% of the spec spent
///     restating byte-identical per-crate edges across all six fleet
///     targets — only cfg/platform-gated crates land in `overrides`.
pub const SCHEMA_VERSION: u32 = 10;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BuildSpec {
    pub version: u32,
    pub workspace: WorkspaceSpec,
    #[serde(default)]
    pub crates: BTreeMap<String, CrateSpec>,
    /// The workspace's primary buildable crate's key in `crates`. Always
    /// populated — either a single-crate workspace's only member or the
    /// first workspace member by declaration order.
    pub root_crate: String,
    #[serde(default)]
    pub workspace_members: Vec<String>,
    /// Per-workspace-member metadata the Nix flake builder needs
    /// (toolName, repo, bin targets) — keyed by package name. Mirrors
    /// `cargo metadata`'s package.repository + targets without forcing
    /// Nix to re-parse Cargo.toml.
    #[serde(default)]
    pub flake_metadata: BTreeMap<String, MemberFlakeMetadata>,
    /// Schema v5+: per-target resolved dep edges. When present,
    /// substrate's lockfile-builder reads dependencies for a given
    /// triple as `base // overrides[triple]` instead of the per-crate
    /// fields. Eliminates the gen-bootstrap chicken-and-egg —
    /// one committed spec serves every fleet target, no Nix-side
    /// cfg evaluation needed (cargo's resolver does cfg per-target,
    /// once per target, at spec-emission time).
    ///
    /// Schema v10+: the COMPACT shape — `base` carries the per-crate
    /// edges that are byte-identical across every target (the common
    /// case: most crates resolve the same edges on every triple), and
    /// `targets[triple].overrides` carries only the crates that differ
    /// on that triple (cfg/platform-gated deps + features). The Nix
    /// consumer reconstructs the full per-triple crates map as
    /// `base // overrides[triple]`. See `CompactTargetResolves`.
    ///
    /// Old shape (top-level runtime_dependencies / build_dependencies
    /// on CrateSpec) is kept for backward compatibility. Substrate
    /// falls back when target_resolves is None.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_resolves: Option<CompactTargetResolves>,
    /// BLAKE3 hex (64 chars) of the workspace's Cargo.lock content at
    /// emit time. Drives a TWO-SIDED idempotence fast-path:
    ///
    /// 1. **Rust side** (`gen build --if-stale`, `gen check-spec`):
    ///    cheap byte-equal-or-skip via SHA-256 string compare.
    ///
    /// 2. **Nix side** (substrate `lockfile-builder.nix`):
    ///    `builtins.hashFile "sha256"` reads the lock natively at
    ///    eval time, compares to this field — IFD is bypassed
    ///    entirely when fresh. THE 10-100× win for `nix run .#rebuild`
    ///    on a clean fleet, because each consumer's gen-IFD never
    ///    runs at all.
    ///
    /// SHA-256 over BLAKE3 specifically because nix has built-in
    /// `hashFile "sha256"` but no BLAKE3 surface; the perf cost
    /// (~3× slower hash compute) is rounding error on a 50–500 KB
    /// `Cargo.lock`. Single hash, both languages compute it the same
    /// way — drift is impossible by construction.
    ///
    /// Missing on schema < 7 specs; downstream consumers treat
    /// absent hash as "always re-emit" / "always re-IFD" for
    /// backward compat. `gen check-spec` reports `UnhashedSpec`,
    /// which `needs_regen() = true` — operators see the gap
    /// mechanically.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cargo_lock_sha256: Option<String>,
}

/// Per-target resolved dep edges for every crate in the workspace's
/// resolve graph. This is the FULL in-memory shape produced by the
/// resolver (one complete crates map per target). It is NOT the
/// serialized shape — `CompactTargetResolves` is what lands in
/// `Cargo.build-spec.json`. `TargetResolve` is the round-trip pivot:
/// `CompactTargetResolves::from_full` compacts a
/// `IndexMap<triple, TargetResolve>` for emission; `expand` reconstructs
/// it for tests / any full-fidelity consumer.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TargetResolve {
    /// Per-crate edge sets for this target. Keyed by the same
    /// `<name>-<version>` key as BuildSpec.crates.
    #[serde(default)]
    pub crates: BTreeMap<String, CrateTargetEdges>,
}

/// Compact serialized representation of per-target resolves (schema v10).
///
/// `base` holds the per-crate edges that are present in EVERY target with
/// byte-identical `CrateTargetEdges` — stored exactly once. `targets`
/// holds, per triple, only the crates that are NOT in `base` (either not
/// universally present, or present-everywhere but differing on some
/// target). The decode contract the Nix consumer obeys is:
///
/// ```text
/// crates(triple) = base // targets[triple].overrides
/// ```
///
/// `base` and `targets` are reserved keys — no target triple is ever
/// named `base` or `targets`, so the shape is unambiguous.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompactTargetResolves {
    /// Per-crate edges identical across every target — stored once.
    #[serde(default)]
    pub base: BTreeMap<String, CrateTargetEdges>,
    /// Per-triple overrides — only the crates that differ from `base`
    /// (or are absent from it) on that triple.
    #[serde(default)]
    pub targets: BTreeMap<String, TargetOverrides>,
}

/// Per-triple override set inside `CompactTargetResolves`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TargetOverrides {
    /// Crates whose edges on this triple differ from (or are absent
    /// from) `base`. Keyed by the same `<name>-<version>` key.
    #[serde(default)]
    pub overrides: BTreeMap<String, CrateTargetEdges>,
}

impl CompactTargetResolves {
    /// Split a full per-target resolve map into the compact base +
    /// overrides representation.
    ///
    /// Algorithm (rust-for-logic, computed once at emit time):
    /// - `base` = every crate key present in EVERY target with a
    ///   byte-identical `CrateTargetEdges` value across all targets.
    ///   The single shared copy is taken from the first target.
    /// - `overrides[T]` = every crate present in target `T` that is NOT
    ///   in `base` (i.e. either not universally-present, or
    ///   present-everywhere-but-differs-on-some-target).
    ///
    /// Lossless by construction: for every triple `T`,
    /// `base // overrides[T]` reconstructs exactly the original crates
    /// map for `T` (same key set, same edges per key). Proven by
    /// `expand`'s round-trip test.
    ///
    /// Single-target input: `base` = all of that target's crates,
    /// every `overrides` empty — still correct, still lossless.
    #[must_use]
    pub fn from_full(full: IndexMap<String, TargetResolve>) -> Self {
        // Empty input → empty compact form (no targets).
        let Some((_first_triple, first_resolve)) = full.iter().next() else {
            return Self::default();
        };

        // A crate is universal+identical iff: it appears in every
        // target, and its edges are equal to the first target's copy
        // in every target. Iterate the first target's crates as the
        // candidate set (a crate absent from the first target can't be
        // present in EVERY target). The per-target loop below visits
        // every target, so `present_in_all` implies true universality.
        let mut base: BTreeMap<String, CrateTargetEdges> = BTreeMap::new();
        for (key, first_edges) in &first_resolve.crates {
            let present_in_all = full
                .values()
                .all(|resolve| resolve.crates.get(key) == Some(first_edges));
            if present_in_all {
                base.insert(key.clone(), first_edges.clone());
            }
        }

        // overrides[T] = crates in T not captured by base.
        let mut targets: BTreeMap<String, TargetOverrides> = BTreeMap::new();
        for (triple, resolve) in &full {
            let mut overrides: BTreeMap<String, CrateTargetEdges> = BTreeMap::new();
            for (key, edges) in &resolve.crates {
                if !base.contains_key(key) {
                    overrides.insert(key.clone(), edges.clone());
                }
            }
            targets.insert(triple.clone(), TargetOverrides { overrides });
        }

        Self { base, targets }
    }

    /// Reconstruct the full per-target resolve map: for every triple,
    /// `base // overrides[triple]` (overrides win on key collision,
    /// though by construction `base` and `overrides` never share a
    /// key). Inverse of `from_full`.
    #[must_use]
    pub fn expand(&self) -> IndexMap<String, TargetResolve> {
        let mut out: IndexMap<String, TargetResolve> = IndexMap::new();
        for (triple, target_overrides) in &self.targets {
            // base first (so override entries that share a key win),
            // then overrides.
            let mut crates = self.base.clone();
            for (key, edges) in &target_overrides.overrides {
                crates.insert(key.clone(), edges.clone());
            }
            out.insert(triple.clone(), TargetResolve { crates });
        }
        out
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CrateTargetEdges {
    /// Per-target Normal ∪ Build union. Same rationale as
    /// `CrateSpec.dependencies` — NOT serialized into
    /// `Cargo.build-spec.json` (schema v9+) because the Nix consumer
    /// reads only `runtime_dependencies` + `build_dependencies` inside
    /// `target_resolves[triple].crates[key]`. This was the single
    /// largest redundant block in the spec (the union restated across
    /// all six fleet targets). `#[serde(default)]` keeps the round-trip
    /// deserialize working; reconstructable as the union of the two
    /// split lists if ever needed.
    #[serde(default, skip_serializing)]
    pub dependencies: Vec<CrateDepSpec>,
    #[serde(default)]
    pub runtime_dependencies: Vec<CrateDepSpec>,
    #[serde(default)]
    pub build_dependencies: Vec<CrateDepSpec>,
    /// Per-target resolved feature list. Cargo's resolver computes
    /// features differently per target (cfg-conditional feature
    /// activation, target-specific dep feature unification). The
    /// top-level `CrateSpec.features` field is whichever target was
    /// processed first during multi-target emission, but it's NOT
    /// correct for other targets — passing macos_fsevent to rustc
    /// on linux is the canonical bug this field eliminates.
    ///
    /// Substrate reads `target_resolves[triple].crates[key].features`
    /// when available, falling back to `spec.crates[key].features`
    /// for backward compat with schema < 5 specs.
    #[serde(default)]
    pub features: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MemberFlakeMetadata {
    /// Default binary name — first `[[bin]]`'s name or, if no explicit
    /// bin section, the package name (cargo's default-bin rule).
    #[serde(default)]
    pub default_bin: Option<String>,
    /// `owner/name` parsed from `[package].repository`. None when the
    /// member doesn't declare a repository — consumer must override.
    #[serde(default)]
    pub repo: Option<String>,

    /// **Substrate-ready module-trio spec.** When `[package.metadata.pleme]`
    /// is set in `Cargo.toml`, gen reads it, applies all defaults
    /// IN RUST (not in Nix), and emits this complete struct.
    /// Substrate's `rust.tool` consumes it as a single attrset
    /// passed directly to `mkModuleTrio` — no nix-side field-by-field
    /// scraping, no defaulting in Nix.
    ///
    /// `None` when the consumer didn't author `[package.metadata.pleme]`.
    /// Substrate emits no module trio in that case (unless the consumer
    /// passed an explicit `module = { ... }` arg, in which case the
    /// flake.nix surface wins).
    ///
    /// Authoring in Cargo.toml:
    ///
    ///   [package.metadata.pleme]
    ///   hm-namespace = "blackmatter.components"
    ///   hm-leaf      = "cli"
    ///   description  = "..."
    ///   binary-name  = "blackmatter-cli"
    ///   package-attr = "blackmatter-cli"
    ///   with-mcp           = false
    ///   with-http          = false
    ///   with-system-daemon = false
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module_trio: Option<ModuleTrioSpec>,
}

/// Complete, ready-to-pass module-trio configuration. All defaults
/// applied IN RUST; Nix consumes the struct VERBATIM (no renames, no
/// defaulting, no logic). Serialized in camelCase to match Substrate's
/// `mkModuleTrio` API — substrate's nix side passes the parsed JSON
/// directly to `mkModuleTrio`, zero work in Nix.
///
/// This is the central-control-plane shape: changes to defaults land
/// in gen-cargo once and propagate to every consumer's next spec
/// emission. The Substrate Nix code has zero TOML knowledge AND zero
/// per-field translation logic.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleTrioSpec {
    /// Final HM option leaf name. From `hm-leaf` or defaults to the
    /// binary's tool id (`default_bin or package_name`).
    pub name: String,
    /// Operator-facing one-line description. From `description` or
    /// falls back to cargo `[package].description`.
    pub description: String,
    /// Overlay attribute name. From `package-attr` or defaults to
    /// the tool id.
    pub package_attr: String,
    /// Binary name in `${package}/bin/`. From `binary-name` or
    /// defaults to the tool id.
    pub binary_name: String,
    /// HM option-tree path the trio mounts under. From `hm-namespace`
    /// or defaults to `"programs"`.
    pub hm_namespace: String,
    /// MCP subcommand wrapper toggle.
    pub with_mcp: bool,
    /// HTTP subcommand service wrapper toggle.
    pub with_http: bool,
    /// System-level daemon wrapper toggle.
    pub with_system_daemon: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkspaceSpec {
    pub root: String,
    #[serde(default)]
    pub members: Vec<WorkspaceMemberSpec>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkspaceMemberSpec {
    pub name: String,
    /// Path relative to the workspace root.
    pub relative_path: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CrateSpec {
    pub name: String,
    pub version: String,
    pub edition: String,
    pub source: CrateSource,
    #[serde(default)]
    pub features: Vec<String>,
    pub proc_macro: bool,
    /// Path to the crate's `build.rs` relative to the unpacked
    /// source root, when one exists. `Some("build/build.rs")` for
    /// crates that place their build script outside the root (e.g.
    /// rustversion); `Some("build.rs")` for the common case; `None`
    /// when there's no build script.
    ///
    /// buildRustCrate auto-detects `build.rs` at the source root,
    /// but doesn't search subdirectories — so passing this
    /// explicitly is the only correct way to surface non-standard
    /// layouts. Without this field, rustversion-style crates fail
    /// at link time with "no such file" for the build-script
    /// output.
    #[serde(default)]
    pub build_script: Option<String>,
    /// `[package] links = "<symbol>"` declaration. Cargo passes this
    /// through as `CARGO_MANIFEST_LINKS` and ring's build.rs (and the
    /// whole `*-sys` family — bzip2-sys, libsqlite3-sys, openssl-sys,
    /// libz-sys, …) asserts on it. nixpkgs' buildRustCrate honors a
    /// `links` arg verbatim. Emitting this from spec avoids per-crate
    /// `links = ...` overrides in pleme-crate-overrides.nix.
    #[serde(default)]
    pub links: Option<String>,
    /// Typed quirks for known third-party upstream crates whose
    /// buildRustCrate compile fails without a class-helper fix.
    /// Source of truth is the const registry in `quirks::REGISTRY`;
    /// gen-cargo emits per-crate quirks into the spec so the substrate
    /// consumer only needs three mechanical dispatch arms (one per
    /// variant), not per-crate Nix-attrset knowledge. Adding a new
    /// quirk = one entry in the typed registry; new quirk classes =
    /// one new enum variant.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quirks: Vec<crate::quirks::CrateQuirk>,
    /// Declared binary targets — empty when the crate is library-only.
    /// Threads through to buildRustCrate's `crateBin` arg, preventing
    /// it from auto-discovering example/test bins under src/bin/ that
    /// don't compile in isolation (e.g. alloc-no-stdlib's heap_alloc).
    /// Each entry: { name, path } where path is relative to the
    /// unpacked source root.
    #[serde(default)]
    pub binaries: Vec<CrateBinSpec>,
    /// Library target when the crate exposes one. None for bin-only or
    /// build-script-only crates. Carries the crate-name override (e.g.
    /// `bzip2_sys` for bzip2-sys) AND the lib path override (e.g.
    /// `lib.rs` instead of the default `src/lib.rs`). Without this,
    /// buildRustCrate's auto-discovery misses crates that put their
    /// library at the root or rename it.
    #[serde(default)]
    pub lib_target: Option<LibTargetSpec>,
    /// All non-dev deps (Normal ∪ Build). Internal derivation source:
    /// `runtime_dependencies` (kind == Normal) and `build_dependencies`
    /// (kind == Build) are filtered FROM this field at emit time, and
    /// the in-Rust invariants (`invariants::check_unresolved_deps`)
    /// walk it before serialization. NOT serialized into
    /// `Cargo.build-spec.json` (schema v9+): the Nix consumer
    /// (`substrate/lib/build/rust/lockfile-builder.nix`) reads ONLY
    /// `runtime_dependencies` + `build_dependencies`, so emitting the
    /// union was pure redundant weight (it restated those two lists a
    /// third time). The union is trivially reconstructable as
    /// `runtime_dependencies ∪ build_dependencies` if a future
    /// cross-tool consumer (e.g. an SBOM emitter) needs it.
    /// `#[serde(default)]` keeps deserialization of newly-emitted
    /// specs working — `diff_specs` (the only full-`BuildSpec`
    /// deserialize path) never reads this field, so an empty Vec on
    /// the read side is correct, not a silent bug.
    #[serde(default, skip_serializing)]
    pub dependencies: Vec<CrateDepSpec>,
    /// Pre-split: deps with kind == "normal". The substrate consumer
    /// passes this list straight to buildRustCrate's `dependencies`
    /// arg — no Nix-side filtering.
    #[serde(default)]
    pub runtime_dependencies: Vec<CrateDepSpec>,
    /// Pre-split: deps with kind == "build". Threads into
    /// buildRustCrate's `buildDependencies`.
    #[serde(default)]
    pub build_dependencies: Vec<CrateDepSpec>,
    /// Pre-shaped crateRenames table — keyed by canonical
    /// published-name, valued as `[{ version, rename }]` records.
    /// Threads through to buildRustCrate's `crateRenames` arg
    /// verbatim. Nix doesn't do any synthesis here.
    #[serde(default)]
    pub crate_renames: BTreeMap<String, Vec<CrateRenameRecord>>,
    /// Pre-computed kwarg attrset for nixpkgs `buildRustCrate`.
    /// Field names match buildRustCrate's exact arg names so that the
    /// substrate consumer is a pure spread — no per-field
    /// `if-then-else` shape-mapping in Nix. Absent fields are skipped
    /// at serialization time so the consumer sees the same "field
    /// missing ⇒ default" semantics it would on a hand-built attrset.
    /// Fields populated here: procMacro, build, links, libName, libPath,
    /// crateName, version, edition, features, crateRenames, release.
    /// Fields NOT populated (the substrate fills these in because they
    /// reference other built derivations or src-path resolution):
    /// `src`, `dependencies`, `buildDependencies`, `crateBin`.
    #[serde(default, skip_serializing_if = "BuildRustCrateArgs::is_empty")]
    pub build_rust_crate_args: BuildRustCrateArgs,
}

// AUDIT (schema v9): `build_rust_crate_args` already pairs default+skip,
// as do `quirks`, `dependencies`, `target_resolves`, and
// `cargo_lock_sha256`. The fields with `skip_serializing_if` that were
// MISSING `default` — the round-trip bug class — were every member of
// `BuildRustCrateArgs` (fixed above). `CrateSpec`'s always-emitted
// fields (`name`/`version`/`edition`/`source`/`features`/`proc_macro`/
// `binaries`/`runtime_dependencies`/`build_dependencies`/`crate_renames`/
// `lib_target`/`build_script`/`links`) are never skipped, so they're
// safe; the `Option`/`Vec`/`IndexMap` among them gain `default` below
// defensively so a hand-trimmed or partial spec still deserializes.

/// Pre-shaped attrset that the substrate consumer spreads directly
/// into `buildRustCrate { … }`. Field names match buildRustCrate's
/// `mkArgs` signature verbatim (camelCase). Optional fields are
/// emitted-iff-present so consumers see absence as "use default."
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BuildRustCrateArgs {
    // ROUND-TRIP INVARIANT (schema v9+): every field here pairs
    // `skip_serializing_if` with `#[serde(default)]`. The skip omits
    // empty fields on serialize; the default supplies the empty value
    // on deserialize when the field is absent. Without the default,
    // `from_slice::<BuildSpec>(to_vec(&spec))` errors with
    // `missing field <name>` for any spec where the field was empty
    // (the documented `crateRenames` regression). Asserted forever by
    // `strict_pipeline::roundtrip_lossless`.
    #[serde(rename = "crateName", default, skip_serializing_if = "Option::is_none")]
    pub crate_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edition: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub features: Vec<String>,
    #[serde(rename = "crateRenames", default, skip_serializing_if = "BTreeMap::is_empty")]
    pub crate_renames: BTreeMap<String, Vec<CrateRenameRecord>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release: Option<bool>,
    #[serde(rename = "procMacro", default, skip_serializing_if = "Option::is_none")]
    pub proc_macro: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub links: Option<String>,
    #[serde(rename = "libName", default, skip_serializing_if = "Option::is_none")]
    pub lib_name: Option<String>,
    #[serde(rename = "libPath", default, skip_serializing_if = "Option::is_none")]
    pub lib_path: Option<String>,
    /// Pre-rustc shell snippet. Set for EVERY crate to export
    /// `CARGO_CRATE_NAME` (cargo's standard env, which buildRustCrate
    /// otherwise omits — rmcp's `env!("CARGO_CRATE_NAME")` and any
    /// future crate that reads the same env now Just Works without a
    /// per-crate override). Caller overrides should APPEND to this
    /// (not replace) to preserve the export.
    #[serde(rename = "preBuild", default, skip_serializing_if = "Option::is_none")]
    pub pre_build: Option<String>,
}

impl BuildRustCrateArgs {
    fn is_empty(&self) -> bool {
        self.crate_name.is_none()
            && self.version.is_none()
            && self.edition.is_none()
            && self.features.is_empty()
            && self.crate_renames.is_empty()
            && self.release.is_none()
            && self.proc_macro.is_none()
            && self.build.is_none()
            && self.links.is_none()
            && self.lib_name.is_none()
            && self.lib_path.is_none()
            && self.pre_build.is_none()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CrateRenameRecord {
    pub version: String,
    pub rename: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CrateBinSpec {
    pub name: String,
    /// Path relative to the unpacked source root (e.g. "src/main.rs"
    /// or "src/bin/cli.rs").
    pub path: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LibTargetSpec {
    /// rustc crate name — `bzip2_sys` for bzip2-sys (cargo replaces `-`
    /// with `_` and honors explicit `[lib].name`).
    pub name: String,
    /// Path to the library's root module relative to the unpacked
    /// source root. `src/lib.rs` for most crates; `lib.rs`, `src/foo.rs`,
    /// etc. when `[lib].path` is set.
    pub path: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum CrateSource {
    /// Registry tarball — fetchurl resolved at eval.
    Registry {
        url: String,
        sha256: String,
        /// File extension hint for nix's unpacker (`.tar.gz`).
        name_with_ext: String,
    },
    /// Git source — fetchgit resolved at eval.
    Git {
        url: String,
        rev: String,
        // `sha256` is prefetched before emission, but a hand-trimmed or
        // pre-prefetch spec may omit it — `default` keeps the round-trip
        // lossless (serde does NOT auto-default `Option` in struct
        // variants).
        #[serde(default)]
        sha256: Option<String>,
    },
    /// Path source (workspace member) — relative to workspace root.
    Path { relative_path: String },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CrateDepSpec {
    /// Consumer-side identifier (`extern crate <name>`); equal to
    /// `package_key` after `/version` stripping when no rename.
    pub name: String,
    /// Key into BuildSpec.crates — uniquely identifies the resolved
    /// dep. Format: `<canonical-name>-<version>`.
    pub package_key: String,
    pub kind: DepKind,
    #[serde(default)]
    pub features: Vec<String>,
    pub uses_default_features: bool,
    pub optional: bool,
    #[serde(default)]
    pub target: Option<String>,
    /// Typed tree placement (#12 — substrate dispatch in Rust, not Nix).
    ///
    /// Each resolved dep edge declares whether the dep is consumed
    /// from the TARGET tree (built for the workload's arch — e.g.
    /// `x86_64-unknown-linux-musl` for rio) or the HOST tree (built
    /// for the build machine's arch — e.g. `aarch64-apple-darwin`
    /// for cid). The substrate's lockfile-builder reads this field
    /// directly instead of reconstructing the placement from
    /// `proc_macro` + dep-kind in Nix.
    ///
    /// Rules:
    /// - `kind = Build` (build.rs deps)        → Host
    /// - `kind = Normal` + target's `proc_macro` → Host
    /// - `kind = Normal` + target not procmacro → Target
    /// - `kind = Dev`                          → Host (deferred; today
    ///                                            dev deps are dropped
    ///                                            from runtime/build
    ///                                            graphs)
    ///
    /// Defaults to Target on deserialize for compat with v3 specs
    /// (#[serde(default)] picks Target as the zero variant).
    #[serde(default)]
    pub tree: BuildTree,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum BuildTree {
    /// Built for the workload's arch (musl/linux/darwin). Default —
    /// most runtime deps land here.
    #[default]
    Target,
    /// Built for the build-machine's arch. Proc-macros, build.rs
    /// scripts, and anything `kind = Build` go here.
    Host,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum DepKind {
    Normal,
    Build,
    Dev,
}

/// Host target triple — the platform the spec targets unless the
/// caller overrides via `generate_for_target`.
fn host_target_triple() -> &'static str {
    // Conservative: known fleet hosts. Full triple detection lives
    // in shikumi config (M+1 enrichment).
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "aarch64-apple-darwin"
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        "x86_64-apple-darwin"
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        "x86_64-unknown-linux-gnu"
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        "aarch64-unknown-linux-gnu"
    }
    #[cfg(not(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64"),
    )))]
    {
        ""
    }
}

/// Generate the complete typed BuildSpec for the workspace at `root`,
/// targeting the host. cargo metadata is invoked with
/// Generates the BuildSpec for the operator's HOST platform.
///
/// `cargo metadata` is invoked with `--filter-platform=<host>` so the
/// resolve graph contains only deps active for this target. Substrate's
/// Nix side consumes the resolved dep edges directly — no Nix-side cfg
/// evaluation, no risk of getting cfg-target right on every nested
/// conditional.
///
/// True multi-platform emission (one spec covering all fleet targets)
/// is task #25 — it requires running cargo metadata N times (once
/// per target) and unioning the dep edges. Until that lands, committed
/// specs are single-target; cross-platform bootstrap relies on
/// substrate's IFD auto-regen to refresh the spec for the target
/// platform.
///
/// Earlier code briefly tried `target = ""` (no --filter-platform).
/// That turned out to STILL be host-filtered by cargo's default
/// behavior, AND it dropped some genuinely cfg-conditional edges
/// (cpufeatures + core_foundation_sys + libc on darwin) — same
/// cfg-resolution problem from a different angle. Revert to the
/// explicit host-filter default; the multi-target fix is #25.
pub fn generate(root: &Path) -> Result<BuildSpec> {
    generate_for_target(root, host_target_triple())
}

/// Canonical fleet targets — pleme-io's primary distribution surfaces.
/// These are the targets `generate_multi_target` resolves dep edges
/// for. New targets land here when the fleet adds an arch.
pub const FLEET_TARGETS: &[&str] = &[
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "x86_64-unknown-linux-gnu",
    "x86_64-unknown-linux-musl",
    "aarch64-unknown-linux-gnu",
    "aarch64-unknown-linux-musl",
];

/// Generates a single BuildSpec containing per-target dep resolves
/// for every fleet target. `cargo metadata` runs once per target via
/// `generate_for_target`; the union populates the spec's
/// `target_resolves` field; crate-level fields (per-crate runtime_/
/// build_dependencies) retain the operator's HOST target for
/// backward-compat (substrate falls back to them when
/// `target_resolves[currentTarget]` is missing).
///
/// This is the right shape for committed `Cargo.build-spec.json`:
/// one spec serves every fleet target without re-running gen. The
/// gen-bootstrap chicken-and-egg (gen's own committed spec being
/// host-filtered, blocking cross-platform build of gen-cli) ends here.
pub fn generate_multi_target(root: &Path) -> Result<BuildSpec> {
    use rayon::prelude::*;

    // Per-target cargo-metadata is independent — each target spawns
    // its own subprocess with `--filter-platform=<triple>`, hits its
    // own dep graph, parses its own JSON. Parallelize across rayon's
    // default thread pool (typically num_cpus). Cold-cache cost for
    // 6 FLEET_TARGETS drops from sequential (6× metadata-seconds) to
    // max(per-target-seconds) + small overhead — a ~5× speedup on
    // typical 8-core machines. Hot-cache (--if-stale fast-path) is
    // unaffected; this only matters when a regen actually fires.
    //
    // `per_target` is an INTERMEDIATE IndexMap (never serialized directly):
    // collected into a Vec keyed by the FLEET_TARGETS index, then folded in
    // target order, so its iteration order is FLEET_TARGETS-deterministic
    // (platform-INDEPENDENT) by construction. The SERIALIZED maps it feeds
    // (CompactTargetResolves.{base,targets,overrides}, BuildSpec.crates) are
    // BTreeMap — canonical key order regardless of this fold order. So the
    // emitted spec is cross-platform byte-stable; this intermediate's order
    // only governs which target is `first` in from_full's universe pick.
    let per_target_vec: Vec<(String, BuildSpec)> = FLEET_TARGETS
        .par_iter()
        .map(|target| {
            eprintln!("gen build: resolving for {}", target);
            generate_for_target(root, target)
                .map(|spec| (target.to_string(), spec))
        })
        .collect::<Result<Vec<_>>>()?;
    let mut per_target: IndexMap<String, BuildSpec> = IndexMap::new();
    for (k, v) in per_target_vec {
        per_target.insert(k, v);
    }

    // Pick the host target's spec as the BASE (crate-level fields,
    // backward-compat). Falls back to the first target if host isn't
    // in FLEET_TARGETS.
    let host = host_target_triple();
    let (_, mut base) = per_target
        .iter()
        .find(|(t, _)| *t == host)
        .map(|(t, s)| (t.clone(), s.clone()))
        .unwrap_or_else(|| {
            per_target
                .iter()
                .next()
                .map(|(t, s)| (t.clone(), s.clone()))
                .expect("FLEET_TARGETS must be non-empty")
        });

    // Union the crates universe across all per-target specs. Per-target
    // resolves may include crates that the host resolve omits (e.g.
    // mio on linux-only paths that don't apply to darwin).
    for spec in per_target.values() {
        for (key, crate_spec) in &spec.crates {
            base.crates.entry(key.clone()).or_insert_with(|| crate_spec.clone());
        }
    }

    // Union per-target crate_renames into the base CrateSpec's
    // crate_renames map. Cargo's per-target resolver may rename a
    // dep on one target that's absent from the host target — e.g.
    // winit on linux has `smithay-client-toolkit -> sctk`; on macos
    // the dep isn't pulled in so the host-only spec's renames are
    // missing that entry. Without the union, substrate's
    // buildRustCrate would compile winit on linux without `--extern
    // sctk=...`, failing with E0433 'unresolved module sctk'.
    //
    // The merge is per-crate, per-canonical-name: deduplicate
    // {version, rename} records so cross-target re-emits aren't
    // counted twice. Each base.crates entry's crate_renames field
    // gets the union of every per-target spec's renames for that
    // same crate.
    for spec in per_target.values() {
        for (key, src) in &spec.crates {
            let Some(dst) = base.crates.get_mut(key) else {
                continue;
            };
            for (canonical, records) in &src.crate_renames {
                let entry = dst.crate_renames.entry(canonical.clone()).or_default();
                for record in records {
                    if !entry.iter().any(|r| {
                        r.version == record.version && r.rename == record.rename
                    }) {
                        entry.push(record.clone());
                    }
                }
            }
            // Mirror the same union into build_rust_crate_args.crate_renames
            // (the pre-shaped buildRustCrate kwargs the substrate consumer
            // spreads verbatim).
            for (canonical, records) in &src.build_rust_crate_args.crate_renames {
                let entry = dst
                    .build_rust_crate_args
                    .crate_renames
                    .entry(canonical.clone())
                    .or_default();
                for record in records {
                    if !entry.iter().any(|r| {
                        r.version == record.version && r.rename == record.rename
                    }) {
                        entry.push(record.clone());
                    }
                }
            }
        }
    }

    // Populate target_resolves with each target's per-crate edges
    // AND per-target features (cargo's resolver computes features
    // differently per target due to cfg-conditional feature
    // activations; the top-level CrateSpec.features field is only
    // correct for whichever target was processed first — see the
    // notify+macos_fsevent leak that prompted this field).
    let mut resolves: IndexMap<String, TargetResolve> = IndexMap::new();
    for (target, spec) in &per_target {
        let mut crates: BTreeMap<String, CrateTargetEdges> = BTreeMap::new();
        for (key, crate_spec) in &spec.crates {
            crates.insert(
                key.clone(),
                CrateTargetEdges {
                    dependencies: crate_spec.dependencies.clone(),
                    runtime_dependencies: crate_spec.runtime_dependencies.clone(),
                    build_dependencies: crate_spec.build_dependencies.clone(),
                    features: crate_spec.features.clone(),
                },
            );
        }
        resolves.insert(target.clone(), TargetResolve { crates });
    }
    // Compact the full per-target map for emission (schema v10):
    // crates with cross-target-identical edges collapse into `base`
    // (stored once); only differing crates land in per-target
    // `overrides`. Lossless — substrate decodes `base // overrides[T]`.
    base.target_resolves = Some(CompactTargetResolves::from_full(resolves));

    Ok(base)
}

/// Generate the BuildSpec for an explicit target triple. Used by
/// cross-build CI to produce a per-target spec.
pub fn generate_for_target(root: &Path, target: &str) -> Result<BuildSpec> {
    // Canonicalize the root path so the relative-path math against
    // cargo metadata's absolute paths produces the right answer.
    let root = std::fs::canonicalize(root).map_err(|source| CargoError::Io {
        path: root.to_path_buf(),
        source,
    })?;
    let root = root.as_path();
    let manifest_path = root.join("Cargo.toml");
    if !manifest_path.exists() {
        return Err(CargoError::Io {
            path: manifest_path,
            source: std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no Cargo.toml at workspace root",
            ),
        });
    }
    // Filter the resolve graph to deps active for this target.
    // cargo's resolver does the heavy lifting; we just pass --filter-platform.
    //
    // Two operator contexts, two postures:
    //
    // 1. Fleet-sweep on a real machine — `~/.cargo/registry` is
    //    populated; a stale entry or a missing git checkout must NOT
    //    prompt for HTTPS auth (operator gets stuck). Fleet sets
    //    `GEN_CARGO_METADATA_OFFLINE=1` (and `GIT_TERMINAL_PROMPT=0`
    //    propagates), forcing `--offline` + non-interactive git.
    //
    // 2. Substrate IFD inside a nix builder — the sandbox has cargo
    //    plumbing but no `~/.cargo/registry` index; `--offline` would
    //    abort with "no matching package named X". Leaves env unset
    //    → default cargo-metadata behaviour, which the builder
    //    pre-populates correctly.
    let offline_mode = std::env::var_os("GEN_CARGO_METADATA_OFFLINE").is_some();
    if offline_mode {
        // Safety: process-global env var. gen is a short-lived CLI;
        // the mutation is bounded to this process's lifetime and
        // pairs with `--offline` as a single hermetic posture.
        unsafe { std::env::set_var("GIT_TERMINAL_PROMPT", "0") };
    }
    let mut cmd = MetadataCommand::new();
    cmd.manifest_path(&manifest_path);
    let mut opts: Vec<String> = Vec::new();
    if offline_mode {
        opts.push("--offline".to_string());
    }
    if !target.is_empty() {
        opts.push("--filter-platform".to_string());
        opts.push(target.to_string());
    }
    cmd.other_options(opts);
    let meta = cmd.exec().map_err(|e| CargoError::Io {
        path: manifest_path.clone(),
        source: std::io::Error::new(std::io::ErrorKind::Other, e.to_string()),
    })?;

    // Lockfile checksums — cargo-metadata doesn't surface them, so we
    // read Cargo.lock directly + index by (name, version). Surface any
    // parse error rather than silently producing an unbuildable spec.
    let checksums: IndexMap<(String, String), String> = {
        let manifest = crate::parse(root).map_err(|e| {
            eprintln!("gen lock-build: gen_cargo::parse failed: {e}");
            e
        })?;
        manifest
            .lockfile
            .map(|lf| {
                lf.resolved
                    .values()
                    .filter_map(|r| {
                        let h = r.integrity.as_ref()?;
                        let hex = h.strip_prefix("sha256:").unwrap_or(h);
                        Some(((r.id.name.clone(), r.id.version.to_string()), hex.to_string()))
                    })
                    .collect()
            })
            .unwrap_or_default()
    };

    let workspace_root_str = root.display().to_string();

    // Identify workspace members.
    let workspace_member_ids: Vec<_> = meta.workspace_members.iter().collect();
    let workspace_members: Vec<WorkspaceMemberSpec> = workspace_member_ids
        .iter()
        .filter_map(|id| meta.packages.iter().find(|p| &p.id == *id))
        .map(|p| {
            let abs_dir = p.manifest_path.parent().map(|p| p.to_string()).unwrap_or_default();
            let rel = pathdiff_relative(&abs_dir, &workspace_root_str)
                .unwrap_or_else(|| abs_dir.clone());
            WorkspaceMemberSpec {
                name: p.name.to_string(),
                relative_path: if rel.is_empty() { ".".to_string() } else { rel },
            }
        })
        .collect();

    let workspace_member_names: std::collections::HashSet<String> = workspace_members
        .iter()
        .map(|m| m.name.clone())
        .collect();

    // Pre-index resolved features by package id.
    let resolved_features: IndexMap<String, Vec<String>> = meta
        .resolve
        .as_ref()
        .map(|r| {
            r.nodes
                .iter()
                .map(|n| (n.id.repr.clone(), n.features.iter().map(String::from).collect()))
                .collect()
        })
        .unwrap_or_default();

    // Map of all resolved package ids → cargo_metadata Package.
    // Used to look up canonical name+version for each dep edge.
    let by_id: IndexMap<String, &cargo_metadata::Package> = meta
        .packages
        .iter()
        .map(|p| (p.id.repr.clone(), p))
        .collect();

    // Build the per-package dep edges from the resolve graph. Each
    // edge carries (local-name, resolved-pkg-id, dep-kinds-from-graph).
    // dep_kinds is the AUTHORITATIVE source for kind classification —
    // a single declared dep may appear in multiple kinds (normal +
    // dev), and the graph is the only place that distinguishes them
    // unambiguously per resolved edge.
    type DepEdge = (String, String, Vec<cargo_metadata::DepKindInfo>);
    let dep_edges: IndexMap<String, Vec<DepEdge>> = meta
        .resolve
        .as_ref()
        .map(|r| {
            r.nodes
                .iter()
                .map(|n| {
                    let edges: Vec<DepEdge> = n
                        .deps
                        .iter()
                        .map(|d| (d.name.clone(), d.pkg.repr.clone(), d.dep_kinds.clone()))
                        .collect();
                    (n.id.repr.clone(), edges)
                })
                .collect()
        })
        .unwrap_or_default();

    let mut crates: BTreeMap<String, CrateSpec> = BTreeMap::new();
    for pkg in &meta.packages {
        let key = format!("{}-{}", pkg.name, pkg.version);
        let is_member = workspace_member_names.contains(pkg.name.as_str());

        // Edition: cargo-metadata provides it directly.
        let edition = format!("{}", pkg.edition);

        // proc-macro detection from cargo-metadata's targets.
        // TargetKind is a stringly-typed list in cargo_metadata 0.18.
        let proc_macro = pkg
            .targets
            .iter()
            .any(|t| t.kind.iter().any(|k| k == "proc-macro"));

        // Build script path detection. A `custom-build` target's
        // src_path is the absolute path to the build.rs file; we
        // strip the package's manifest_dir prefix to get the
        // relative path the substrate consumer needs.
        let manifest_dir = pkg.manifest_path.parent().map(|p| p.to_string()).unwrap_or_default();
        let build_script = pkg
            .targets
            .iter()
            .find(|t| t.kind.iter().any(|k| k == "custom-build"))
            .and_then(|t| {
                let abs = t.src_path.to_string();
                strip_dir_prefix(&abs, &manifest_dir)
            });

        // `[package] links = "<symbol>"` declaration. cargo-metadata
        // surfaces this directly on the Package — pass through verbatim
        // for the substrate consumer to wire as buildRustCrate's `links`
        // arg, which in turn sets `CARGO_MANIFEST_LINKS` for build.rs
        // assertions (ring's `ring_core_<ver>_` is the canonical case).
        let links: Option<String> = pkg.links.clone();

        // Binary targets — only `bin` kind. Empty list (the common
        // case for libs) prevents buildRustCrate from auto-discovering
        // src/bin/* files that may not compile in isolation.
        let binaries: Vec<CrateBinSpec> = pkg
            .targets
            .iter()
            .filter(|t| t.kind.iter().any(|k| k == "bin"))
            .filter_map(|t| {
                let abs = t.src_path.to_string();
                strip_dir_prefix(&abs, &manifest_dir).map(|path| CrateBinSpec {
                    name: t.name.clone(),
                    path,
                })
            })
            .collect();

        // Library target. cargo represents libs as kind ∈ {lib, rlib,
        // staticlib, cdylib, dylib, proc-macro}. Pick the first
        // non-custom-build one — cargo only allows one library per
        // crate. `target.name` already has the rustc-friendly form
        // (underscores). `src_path` honors `[lib].path` overrides.
        //
        // Lib-target emission rules:
        // - Path-deps (workspace members + external path-deps like
        //   `gen-platform = { path = "../gen/crates/gen-platform" }`):
        //   ALWAYS emit. lockfile-builder's per-tree builder uses
        //   src = workspaceSrc and prefixes lib_target.path with the
        //   path-dep's relative_path. Without lib_target, buildRustCrate's
        //   default `src/lib.rs` auto-discovery resolves against the
        //   workspace root instead of the actual member/path-dep subdir,
        //   producing a drv with NO compiled rlib output (silent failure
        //   that surfaces as "extern location for X does not exist" at
        //   the consumer's rustc invocation).
        // - Registry/git crates at default `src/lib.rs` with default
        //   rustc name: suppress emission. buildRustCrate's auto-discovery
        //   is identical to explicit args here — including the proc-macro
        //   `crate-type = ["proc-macro", "rlib"]` dual that lets crates
        //   like tatara-lisp-derive co-locate non-proc-macro fn items
        //   with their macros (explicit libName + libPath would force a
        //   proc-macro-only compile that rejects them).
        // - Registry/git crates with overridden path/name (fnv, bzip2-sys,
        //   document-features, …): emit so buildRustCrate finds the lib.
        //
        // The critical distinction: workspace membership is NOT the right
        // discriminator — `is_member` is true only for crates listed in
        // the current workspace's `[workspace] members`. External path-deps
        // (cargo's `path = "../foo"` form) have `pkg.source = None` like
        // members do, but ARE NOT members; they were previously suppressed
        // by `!is_member`, causing them to produce empty drvs. The fix:
        // use `pkg.source.is_none()` as the path-dep predicate, which is
        // true for both members and external path-deps.
        let is_path_dep = pkg.source.is_none();
        let lib_target = pkg
            .targets
            .iter()
            .find(|t| {
                t.kind.iter().any(|k| {
                    matches!(
                        k.as_str(),
                        "lib" | "rlib" | "staticlib" | "cdylib" | "dylib" | "proc-macro"
                    )
                })
            })
            .and_then(|t| {
                let abs = t.src_path.to_string();
                let path = strip_dir_prefix(&abs, &manifest_dir)?;
                let default_name = pkg.name.as_str().replace('-', "_");
                let default_path = "src/lib.rs";
                let is_default = t.name == default_name && path == default_path;
                if is_default && !is_path_dep {
                    return None;
                }
                Some(LibTargetSpec {
                    name: t.name.clone(),
                    path,
                })
            });

        // Source resolution.
        // Path-dep dispatch — two distinct subcases:
        //
        // 1. **Workspace member** (path is inside the workspace root):
        //    `Path { relative_path }` where `relative_path` is the
        //    subdir under workspace root. Substrate's lockfile-builder
        //    finds the source at `<workspaceSrc>/<relative_path>`.
        //
        // 2. **External path-dep** (path escapes the workspace via
        //    `..`): the sibling repo's source is NOT in the nix build
        //    sandbox (src = workspaceSrc). Emitting `Path { relative_path = "../foo" }`
        //    fails closed in substrate's lockfile-builder L143.
        //
        //    Instead, ask the typed `PathDepResolver` to convert the
        //    sibling directory into a `(git_url, rev)` pair (production
        //    impl reads the sibling's git remote + HEAD; tests inject
        //    a `MockResolver`). On success emit `Git { url, rev }`;
        //    on failure return a typed `UnresolvableExternalPath`
        //    error pointing the operator at the manual fix.
        //
        //    The auto-convert means operators can keep `path = "../"`
        //    in Cargo.toml for local-dev ergonomics; the EMITTED spec
        //    is always nix-safe.
        let source = if is_path_dep {
            let abs_dir = pkg.manifest_path.parent().map(|p| p.to_string()).unwrap_or_default();
            let rel = pathdiff_relative(&abs_dir, &workspace_root_str)
                .or_else(|| relative_path_escaping(&abs_dir, &workspace_root_str))
                .unwrap_or_else(|| abs_dir.clone());
            // External if the resolved relative path either escapes
            // via `..` OR remains absolute (which means pathdiff +
            // relative_path_escaping both failed). Either way the
            // source lives outside the workspace and must auto-convert
            // to a git source for nix-sandbox compatibility.
            let is_external = rel.starts_with("..") || rel.starts_with('/');
            if is_external {
                let resolver = crate::path_resolver::GitCliResolver;
                use crate::path_resolver::PathDepResolver;
                let abs_path = std::path::Path::new(&abs_dir);
                match resolver.resolve(abs_path) {
                    Some((url, rev)) => CrateSource::Git { url, rev, sha256: None },
                    None => {
                        return Err(CargoError::UnresolvableExternalPath {
                            name: pkg.name.to_string(),
                            version: pkg.version.to_string(),
                            abs_dir: std::path::PathBuf::from(abs_dir.clone()),
                            workspace_root: std::path::PathBuf::from(workspace_root_str.clone()),
                            reason: format!(
                                "directory at `{abs_dir}` either is not a git repository, has no `origin` remote, or its remote URL is not a GitHub URL (only github.com auto-resolution is implemented today)"
                            ),
                        });
                    }
                }
            } else {
                CrateSource::Path {
                    relative_path: if rel.is_empty() { ".".to_string() } else { rel },
                }
            }
        } else if let Some(src) = &pkg.source {
            let src_str = src.to_string();
            if src_str.starts_with("registry+") {
                let sha = checksums
                    .get(&(pkg.name.to_string(), pkg.version.to_string()))
                    .cloned()
                    .unwrap_or_default();
                if sha.is_empty() {
                    eprintln!(
                        "gen lock-build: missing checksum for {}/{}",
                        pkg.name, pkg.version
                    );
                }
                CrateSource::Registry {
                    // static.crates.io is the canonical immutable mirror
                    // cargo itself fetches from. The /api/v1/.../download
                    // endpoint is rate-limited and frequently 403's against
                    // nix's fetchurl user-agent — use the CDN directly.
                    url: format!(
                        "https://static.crates.io/crates/{}/{}-{}.crate",
                        pkg.name, pkg.name, pkg.version
                    ),
                    sha256: sha,
                    name_with_ext: format!("{}-{}.tar.gz", pkg.name, pkg.version),
                }
            } else if src_str.starts_with("git+") {
                let trimmed = src_str.trim_start_matches("git+");
                let (raw_url, rev) = trimmed
                    .rsplit_once('#')
                    .map(|(u, f)| (u.to_string(), f.to_string()))
                    .unwrap_or_else(|| (trimmed.to_string(), String::new()));
                // Cargo encodes the requested ref as `?branch=...` /
                // `?tag=...` / `?rev=...` on the URL. Strip it here so
                // the Nix consumer doesn't need to know about it — pure
                // dispatch on a clean URL.
                let url = raw_url.split('?').next().unwrap_or(&raw_url).to_string();
                CrateSource::Git {
                    url,
                    rev,
                    sha256: None,
                }
            } else {
                // Path source not in workspace — bare relative.
                let abs_dir = pkg.manifest_path.parent().map(|p| p.to_string()).unwrap_or_default();
                let rel = pathdiff_relative(&abs_dir, &workspace_root_str)
                    .unwrap_or_else(|| abs_dir.clone());
                CrateSource::Path { relative_path: rel }
            }
        } else {
            CrateSource::Path {
                relative_path: ".".to_string(),
            }
        };

        let features = resolved_features
            .get(&pkg.id.repr)
            .cloned()
            .unwrap_or_default();

        // Build typed dep edges using the resolve graph (authoritative
        // for both "what's in the closure" AND "what kind each edge
        // is"). The graph's dep_kinds field carries the per-edge kind
        // — that's the source of truth, never the Cargo.toml-side
        // declaration. A single declared dep may register as both
        // [dependencies] AND [dev-dependencies] in cargo's eyes;
        // dep_kinds enumerates each one.
        let edges_for_pkg = dep_edges.get(&pkg.id.repr).cloned().unwrap_or_default();

        let mut dependencies: Vec<CrateDepSpec> = Vec::new();
        for (local_name, dep_pkg_id, dep_kinds) in &edges_for_pkg {
            let Some(dep_pkg) = by_id.get(dep_pkg_id) else { continue; };

            // Prune phantom optional edges. cargo_metadata's resolve
            // graph lists an OPTIONAL dependency edge even when its
            // gating feature is OFF — the node's resolved `features`
            // list is the source of truth, the `deps` array
            // over-reports. Building such a phantom crate in isolation
            // is actively wrong: its features come out as the crate's
            // bare defaults rather than what the (never-activated)
            // consumer would have selected, so e.g. `sqlx-sqlite`
            // drags in `libsqlite3-sys` which then lands on its
            // prepare_v2-only sqlite-3.6.8 bindings and the compile
            // dies with `E0432: no sqlite3_prepare_v3`. cargo's own
            // build never compiles it (the `sqlite` feature is off);
            // gen must match that. An optional dep is ACTIVE iff its
            // implicit feature name (rename-or-crate-name) is in the
            // parent's resolved features, OR a resolved feature
            // expands to `dep:<name>` (namespaced-feature style).
            // Match the consumer's declaration(s) by crate name (both
            // sides hyphenated — robust against the underscored lib
            // name in `local_name` and against renames). Conservative:
            // only prune when EVERY declaration of the target is
            // optional AND none is activated.
            {
                let decls: Vec<&cargo_metadata::Dependency> = pkg
                    .dependencies
                    .iter()
                    .filter(|d| d.name == dep_pkg.name)
                    .collect();
                let all_optional = !decls.is_empty() && decls.iter().all(|d| d.optional);
                if all_optional {
                    let parent_feats = resolved_features
                        .get(&pkg.id.repr)
                        .map(Vec::as_slice)
                        .unwrap_or(&[]);
                    let activated = decls.iter().any(|d| {
                        let implicit = d.rename.clone().unwrap_or_else(|| d.name.clone());
                        parent_feats.iter().any(|f| f == &implicit)
                            || parent_feats.iter().any(|f| {
                                pkg.features.get(f).is_some_and(|exp| {
                                    exp.iter().any(|e| {
                                        e.strip_prefix("dep:").is_some_and(|n| {
                                            n == implicit || n == d.name
                                        })
                                    })
                                })
                            })
                    });
                    if !activated {
                        // Phantom optional edge — cargo never builds it.
                        continue;
                    }
                }
            }

            // Edge target key: `name-version-rev` for a git dep so it points at
            // the SPECIFIC rev this edge uses (two revs of one version are
            // distinct crates — TOOLCHAIN-FRESHNESS §X.4b.b); `name-version`
            // for registry/path.
            let package_key = match dep_pkg.source.as_ref().map(|s| s.to_string()) {
                Some(s) => match git_rev_of_source(&s) {
                    Some(rev) if !rev.is_empty() => {
                        format!("{}-{}-{}", dep_pkg.name, dep_pkg.version, rev)
                    }
                    _ => format!("{}-{}", dep_pkg.name, dep_pkg.version),
                },
                None => format!("{}-{}", dep_pkg.name, dep_pkg.version),
            };

            // Emit ONE entry per non-Dev kind. A single declared dep can
            // appear as BOTH Normal and Build (e.g. mime_guess uses
            // unicase as a normal dep AND inside its `build.rs` via
            // `extern crate unicase`). Cargo links it twice — once
            // into the lib, once into the build script. If we emit
            // only the first kind, buildRustCrate links it only to
            // the lib, and the build-script compile fails with
            // E0658 (extern crate falls through to the sysroot).
            let kinds_to_emit: Vec<_> = dep_kinds
                .iter()
                .filter(|k| !matches!(k.kind, cargo_metadata::DependencyKind::Development))
                .collect();
            if kinds_to_emit.is_empty() { continue; }

            for graph_kind in &kinds_to_emit {
                let kind = match graph_kind.kind {
                    cargo_metadata::DependencyKind::Normal => DepKind::Normal,
                    cargo_metadata::DependencyKind::Build => DepKind::Build,
                    cargo_metadata::DependencyKind::Development => DepKind::Dev,
                    _ => DepKind::Normal,
                };
                let target = graph_kind.target.as_ref().map(|p| p.to_string());

                // Look up the consumer's declared dep entry to recover
                // features + optional + uses_default_features. Match by
                // local name + kind to avoid mis-attributing a normal
                // edge's features to a dev edge of the same name.
                //
                // The graph edge's `local_name` is cargo's `extern crate`
                // name — `-` normalized to `_` (e.g. `sqlx_sqlite`), or the
                // explicit `rename` when present. The declared dep's name is
                // the manifest spelling (`sqlx-sqlite`). Normalize BOTH sides
                // (`-` → `_`) before comparing so the match doesn't silently
                // fall through to the `None` branch (which would forge
                // `optional = false` onto a genuinely-optional edge — the
                // sqlx-sqlite `optional: false` bug).
                //
                // TARGET-DISAMBIGUATED MATCH. A single dep name + kind can be
                // declared MULTIPLE times in one manifest — once bare under
                // `[dependencies]` and once (or more) under
                // `[target.'cfg(...)'.dependencies]` — and the two declarations
                // can disagree on `optional`. `wgpu` is the canonical case:
                //   [dependencies.wgpu-core]                           # optional=true
                //   [target.'cfg(not(target_arch="wasm32"))'.dependencies.wgpu-core] # optional=false
                // A plain `.find()` on name+kind returns the FIRST (bare,
                // optional) declaration, so its `optional=true` + `target=None`
                // win even when the resolved edge cargo actually pulled is the
                // non-optional `cfg(not(wasm32))` one. The downstream
                // optional-dep activation guard then sees `optional && not
                // target-gated && no feature` → drops the edge entirely. On
                // native targets that silently removed wgpu's wgpu-core /
                // wgpu-hal backend deps, so `wgpu`'s own `backend/wgpu_core.rs`
                // (`use wgc::command::bundle_ffi::*`) failed to compile with 17×
                // E0425 `cannot find function wgpu_render_bundle_draw_indexed_indirect`
                // and siblings — fleet-wide Linux build break (escriba, mado,
                // every GPU consumer).
                //
                // The resolved edge's `target` (`graph_kind.target`) is the
                // authoritative cfg cargo evaluated under --filter-platform.
                // Disambiguate by it: among name+kind matches, prefer the
                // declaration whose `target` string-equals the edge's target;
                // only fall back to a name+kind-only match when no
                // target-aligned declaration exists (the common single-decl
                // case, byte-identical to the old behavior).
                let name_kind_matches = |d: &&cargo_metadata::Dependency| -> bool {
                    let consumer_name = d
                        .rename
                        .clone()
                        .unwrap_or_else(|| d.name.clone())
                        .replace('-', "_");
                    consumer_name == local_name.replace('-', "_")
                        && d.kind == graph_kind.kind
                };
                let declared = pkg
                    .dependencies
                    .iter()
                    .find(|d| {
                        name_kind_matches(d)
                            && d.target.as_ref().map(ToString::to_string) == target
                    })
                    .or_else(|| pkg.dependencies.iter().find(name_kind_matches));
                let (features, uses_default_features, optional) = match declared {
                    Some(d) => (
                        d.features.iter().map(String::from).collect(),
                        d.uses_default_features,
                        d.optional,
                    ),
                    None => (Vec::new(), true, false),
                };

                // WEAK-DEPENDENCY / UNACTIVATED-OPTIONAL FILTER.
                //
                // cargo's `resolve.nodes[].deps` is *over-inclusive* for
                // optional dependencies: it lists every optional dep a
                // package declares, regardless of whether the current
                // feature set actually activates it. `cargo tree` shows
                // only the activated graph, and the cargo-built binary
                // contains only activated optional deps — so an optional
                // edge that no feature turned on must NOT be emitted into
                // the build spec (substrate would otherwise build it, and
                // a never-activated driver like sqlx-sqlite fails the musl
                // image build).
                //
                // The activation decision is the FULL matrix in
                // `optional_dep_activated`: an optional dep is activated iff an
                // ACTIVE feature of the consuming crate (a) names it via
                // `dep:<name>`, (b) references it via a NON-weak `<name>/x`, or
                // (c) the bare implicit feature `<name>` is active. Keying only
                // on the implicit feature is wrong — cargo SUPPRESSES that
                // implicit feature whenever any feature uses `dep:<name>`
                // syntax (the deranged `serde = ["dep:serde_core"]` case),
                // which would wrongly drop a genuinely-activated edge. A weak
                // `<name>?/x` ref does NOT activate (the sqlx-sqlite case).
                if optional {
                    let consuming_feats = resolved_features
                        .get(&pkg.id.repr)
                        .map(|f| f.as_slice())
                        .unwrap_or(&[]);
                    // RENAME-AWARE optional-dep activation name. cargo names
                    // an optional dep — in BOTH the implicit `<name>` feature
                    // AND the `dep:<name>` form — by the CONSUMER-SIDE alias:
                    // the `rename` when the dep declares `package = "..."`,
                    // else the manifest name. The earlier assumption ("always
                    // the manifest name") is wrong and silently dropped renamed
                    // optional edges — attohttpc declares
                    // `rustls_opt_dep = { package = "rustls", optional = true }`
                    // and gates it behind `tls-rustls = ["dep:rustls_opt_dep"]`
                    // + the implicit `rustls-opt-dep` feature, NEITHER of which
                    // names the package `rustls`. Keying on the package name
                    // made `optional_dep_activated` return false → the edge was
                    // dropped → consumer rustc failed with E0463
                    // `can't find crate for rustls_opt_dep`.
                    //
                    // The resolved node-dep `local_name` IS cargo's
                    // rename-aware extern name (cargo metadata's
                    // `resolve.nodes[].deps[].name` already encodes the rename),
                    // so it is the authoritative activation key; the declared
                    // `rename` is an equivalent fallback when the edge has a
                    // matching manifest entry, and the manifest `name` is the
                    // last resort. This aligns gen's activation check with the
                    // same rename-aware edge cargo (and crate2nix's Cargo.nix)
                    // resolve against.
                    let dep_name = declared
                        .and_then(|d| d.rename.clone())
                        .unwrap_or_else(|| local_name.clone());
                    // TARGET-CFG-GATED OPTIONAL DEPS are activated by the
                    // PLATFORM match, NOT a feature. cargo's `--filter-platform`
                    // resolve already evaluated the `[target.'cfg(...)']` guard
                    // and lists the edge only when the target matched — e.g.
                    // rustix's `libc`/`libc_errno` (its libc backend) are
                    // `optional = true` but pulled purely by
                    // `cfg(all(not(windows), any(rustix_use_libc, miri,
                    // not(all(target_os = "linux", ...)))))` matching apple/bsd,
                    // with NO `use-libc` feature in the resolved set. The
                    // feature-only activation check below cannot see this path
                    // and would drop the edge → rustc fails with
                    // `error[E0432]: unresolved import libc` on darwin. So an
                    // optional dep whose matched manifest declaration is
                    // target-scoped (`d.target.is_some()`) and which survived
                    // --filter-platform is trusted as activated. NON-target
                    // optional deps (the sqlx-sqlite feature-gated case) keep
                    // the strict feature filter.
                    // Typed, exhaustive emit/skip decision. The previously-inline
                    // boolean (`!target_gated && !optional_dep_activated(...)`) is
                    // now `classify_optional_dep(...).is_emitted()` — one named
                    // total function over the four activation outcomes (see
                    // `OptionalDepActivation`). We are inside `if optional`, so the
                    // `is_optional` argument is `true`; this guard IS the
                    // `NotOptional` arm. Byte-identical to the old boolean: the
                    // edge is dropped iff classification is `Unactivated`
                    // (optional ∧ not target-gated ∧ no active feature).
                    let is_target_gated = declared.is_some_and(|d| d.target.is_some());
                    if !classify_optional_dep(
                        true,
                        is_target_gated,
                        consuming_feats,
                        &pkg.features,
                        &dep_name,
                    )
                    .is_emitted()
                    {
                        // Unactivated optional dep — skip this edge entirely.
                        continue;
                    }
                }

                // I5: typed BuildTree placement — substrate consumes
                // this directly instead of reconstructing host/target
                // dispatch in Nix from proc_macro + kind. Rules
                // (mirrored in BuildTree's docstring):
                //   - kind=Build (build.rs deps)      → Host
                //   - kind=Normal + dep.proc_macro    → Host
                //   - kind=Normal otherwise           → Target
                //   - kind=Dev                        → Host (dev deps are
                //     filtered out before reaching substrate; placement
                //     is documented for completeness)
                let dep_is_proc_macro = dep_pkg
                    .targets
                    .iter()
                    .any(|t| t.kind.iter().any(|k| k == "proc-macro"));
                let tree = match kind {
                    DepKind::Build | DepKind::Dev => BuildTree::Host,
                    DepKind::Normal if dep_is_proc_macro => BuildTree::Host,
                    DepKind::Normal => BuildTree::Target,
                };
                dependencies.push(CrateDepSpec {
                    name: local_name.clone(),
                    package_key: package_key.clone(),
                    kind,
                    features,
                    uses_default_features,
                    optional,
                    target,
                    tree,
                });
            }
        }

        // Pre-split + pre-shape Nix-side data — substrate consumer
        // receives ready-to-pass typed values, no Nix-side
        // semantic decisions.
        let runtime_dependencies: Vec<CrateDepSpec> = dependencies
            .iter()
            .filter(|d| matches!(d.kind, DepKind::Normal))
            .cloned()
            .collect();
        let build_dependencies: Vec<CrateDepSpec> = dependencies
            .iter()
            .filter(|d| matches!(d.kind, DepKind::Build))
            .cloned()
            .collect();
        let crate_renames =
            synthesize_crate_renames(&runtime_dependencies, &build_dependencies, &by_id);

        // Cargo's resolver may produce multiple distinct nodes that share
        // the same `name-version` key but resolve from different sources
        // (e.g. a git crate with `?branch=main` vs a plain git URL each
        // yield separate resolve nodes for the same crate). The spec keys
        // by name-version, so collapse them while keeping the richer
        // resolution (more features → more conditional deps activated).
        // Without the merge, the later iteration silently overwrites the
        // first one's features AND its deps — tear's `shikumi` losing
        // both its `cli` feature and its `clap` dep is the canonical
        // failure mode.
        // Pre-shape buildRustCrate kwargs in Rust — substrate's
        // lockfile-builder spreads this verbatim. No Nix-side
        // `if-then-else` for every conditional field.
        // rustc crate-name = package name with `-` → `_` (or explicit
        // `[lib].name` override). Setting CARGO_CRATE_NAME universally
        // means every crate that reads `env!("CARGO_CRATE_NAME")` at
        // proc-macro expansion (rmcp 0.15's `src/model.rs:860`,
        // future crates of the same class) Just Works — no per-crate
        // override.
        let rustc_crate_name = lib_target
            .as_ref()
            .map(|t| t.name.clone())
            .unwrap_or_else(|| pkg.name.replace('-', "_"));
        let pre_build = format!("export CARGO_CRATE_NAME={};", rustc_crate_name);

        let build_rust_crate_args = BuildRustCrateArgs {
            crate_name: Some(pkg.name.to_string()),
            version: Some(pkg.version.to_string()),
            edition: Some(edition.clone()),
            features: features.clone(),
            crate_renames: crate_renames.clone(),
            release: Some(true),
            proc_macro: if proc_macro { Some(true) } else { None },
            build: build_script.clone(),
            links: links.clone(),
            lib_name: lib_target.as_ref().map(|t| t.name.clone()),
            lib_path: lib_target.as_ref().map(|t| t.path.clone()),
            pre_build: Some(pre_build),
        };

        // Look up typed quirks from the canonical registry. Lookup
        // is by crate name; empty Vec when no quirks registered.
        let quirks = crate::quirks::for_crate(pkg.name.as_str());

        let new_entry = CrateSpec {
            name: pkg.name.to_string(),
            version: pkg.version.to_string(),
            edition,
            source,
            features,
            proc_macro,
            build_script,
            links,
            quirks,
            binaries,
            lib_target,
            dependencies,
            runtime_dependencies,
            build_dependencies,
            crate_renames,
            build_rust_crate_args,
        };
        // Dedup policy when two packages collide on `${name}-${version}`:
        //   1. WORKSPACE-MEMBER wins. A workspace path package and a
        //      git/registry copy of the same (name, version) can both
        //      appear in `cargo metadata` when a transitive git dep
        //      references the same crate via git. The workspace
        //      member is the source of truth (it has lib_target,
        //      correct manifest_dir, etc.); the git copy is a stale
        //      duplicate that shouldn't overwrite it.
        //   2. Otherwise, prefer the entry with the richer features
        //      set (per-target feature unification picks the union).
        let new_is_workspace = pkg.source.is_none();
        // Git crates key by `name-version-rev` so two revs of one version are
        // DISTINCT entries instead of being deduped into one (the dedup below
        // is for genuine `name-version` collisions: a workspace member vs a
        // registry/path copy). Registry/path crates keep `name-version`.
        // TOOLCHAIN-FRESHNESS §X.4b.b.
        let key = match &new_entry.source {
            CrateSource::Git { rev, .. } if !rev.is_empty() => {
                format!("{}-{}-{}", pkg.name, pkg.version, rev)
            }
            _ => key,
        };
        match crates.get(&key) {
            Some(prev) => {
                let prev_is_workspace = matches!(prev.source, CrateSource::Path { .. });
                match (prev_is_workspace, new_is_workspace) {
                    (true, false) => {
                        // existing is workspace member; new is git/registry copy — keep existing
                    }
                    (false, true) => {
                        // existing is git/registry copy; new is workspace member — replace
                        crates.insert(key, new_entry);
                    }
                    _ if prev.features.len() > new_entry.features.len() => {
                        // existing is richer — keep
                    }
                    _ => {
                        crates.insert(key, new_entry);
                    }
                }
            }
            None => {
                crates.insert(key, new_entry);
            }
        }
    }

    let workspace_member_keys: Vec<String> = workspace_members
        .iter()
        .filter_map(|m| {
            let pkg = meta.packages.iter().find(|p| p.name.as_str() == m.name)?;
            Some(format!("{}-{}", pkg.name, pkg.version))
        })
        .collect();

    // root_crate: cargo's reported root_package when set (single-crate
    // workspaces), else first declared workspace member. Always
    // populated — the Nix consumer treats it as authoritative without a
    // fallback dance.
    let root_crate: String = match meta.root_package() {
        Some(p) => format!("{}-{}", p.name, p.version),
        None => workspace_member_keys
            .first()
            .cloned()
            .ok_or_else(|| CargoError::Io {
                path: manifest_path.clone(),
                source: std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "workspace has no members; gen needs at least one buildable crate",
                ),
            })?,
    };

    // Per-workspace-member flake metadata: tool_name + repo derived
    // here so the Nix consumer doesn't re-parse Cargo.toml. cargo
    // resolves [workspace.package] inheritance into package.repository
    // already; we read the resolved value.
    let mut flake_metadata: BTreeMap<String, MemberFlakeMetadata> = BTreeMap::new();
    for m in &workspace_members {
        let Some(pkg) = meta.packages.iter().find(|p| p.name.as_str() == m.name) else {
            continue;
        };
        let key = format!("{}-{}", pkg.name, pkg.version);
        let Some(c) = crates.get(&key) else { continue };
        let default_bin = c.binaries.first().map(|b| b.name.clone());
        // Parse owner/name from canonical GitHub-style URLs only. Anything
        // else stays None and forces the consumer to override explicitly.
        let repo = pkg.repository.as_deref().and_then(parse_owner_repo);

        // Read `[package.metadata.pleme]` from Cargo.toml — when
        // present, produce a complete `ModuleTrioSpec` with every
        // default applied in Rust. Substrate's nix side consumes the
        // struct verbatim; zero TOML scraping, zero per-field
        // defaulting on the nix side.
        let pleme = pkg
            .metadata
            .as_object()
            .and_then(|o| o.get("pleme"))
            .and_then(|p| p.as_object());
        let module_trio = pleme.map(|p| {
            let str_or = |k: &str, default: String| -> String {
                p.get(k).and_then(|v| v.as_str()).map(String::from).unwrap_or(default)
            };
            let bool_or = |k: &str, default: bool| -> bool {
                p.get(k).and_then(|v| v.as_bool()).unwrap_or(default)
            };
            // Defaults applied here so consumers (nix) get a fully
            // populated, ready-to-pass attrset.
            //
            // Convention: the "tool identity" defaults to the binary's
            // name, NOT the package's name. Most pleme-io tools have
            // identical pkg.name and bin.name, but the divergent cases
            // matter (e.g. `hibikine` package shipping the `hibiki`
            // binary — the HM option, overlay attr, and binary name
            // are all `hibiki`, not `hibikine`). Substrate's pre-
            // smart-read default was `name = toolName` where toolName
            // = default_bin; we replicate that here in Rust.
            let pkg_name = pkg.name.to_string();
            let tool_id = default_bin.clone().unwrap_or_else(|| pkg_name.clone());
            let name = str_or("hm-leaf", tool_id.clone());
            let description = str_or(
                "description",
                pkg.description.clone().unwrap_or_else(|| format!("{tool_id} CLI tool")),
            );
            let package_attr = str_or("package-attr", tool_id.clone());
            let binary_name = str_or("binary-name", tool_id.clone());
            let hm_namespace = str_or("hm-namespace", "programs".to_string());
            ModuleTrioSpec {
                name,
                description,
                package_attr,
                binary_name,
                hm_namespace,
                with_mcp: bool_or("with-mcp", false),
                with_http: bool_or("with-http", false),
                with_system_daemon: bool_or("with-system-daemon", false),
            }
        });

        flake_metadata.insert(
            pkg.name.to_string(),
            MemberFlakeMetadata {
                default_bin,
                repo,
                module_trio,
            },
        );
    }


    // Prefetch sha256 for every Git source so substrate's
    // lockfile-builder can dispatch pkgs.fetchgit with a FIXED HASH
    // (no IFD, no impure builtins.fetchGit). Sequential — could be
    // parallelized but git deps are typically few.
    //
    // Pure-Rust via `GitPrefetcher` trait (gix clone + nix-nar
    // streaming NAR-sha256 + SRI encode). No subprocess, no shell
    // script, no nix-prefetch-git/git/nix on the substrate IFD
    // sandbox PATH. See `theory/RUST-NATIVE-PREFETCH.md` for the
    // design + `git_prefetcher.rs` for the impl.
    //
    // Hard-fail on prefetch error. Earlier this was a swallowed
    // warning: when prefetch couldn't reach the remote (sandbox DNS,
    // transient network), gen continued and emitted a
    // CrateSource::Git with `sha256: None`. substrate's
    // lockfile-builder then generated a `pkgs.fetchgit` call without
    // outputHash — which produces a NON-FOD derivation that's denied
    // network access in the sandbox, and the downstream fleet rebuild
    // then explodes with "Could not resolve host: github.com" deep
    // inside an unrelated build. Surfaced via the rio rebuild on
    // 2026-05-29; root-causing the malformed hayai-731ad59.drv took
    // significantly more time than the 5-line fix to write.
    let prefetcher = crate::git_prefetcher::default_prefetcher();
    for crate_spec in crates.values_mut() {
        if let CrateSource::Git { url, rev, sha256, .. } = &mut crate_spec.source {
            if sha256.is_none() && !url.is_empty() && !rev.is_empty() {
                let hash = prefetcher.prefetch(url, rev).map_err(|e| {
                    CargoError::PrefetchSha256Failed {
                        url: url.clone(),
                        rev: rev.clone(),
                        reason: e.to_string(),
                    }
                })?;
                *sha256 = Some(hash.sri);
            }
        }
    }

    Ok(BuildSpec {
        version: SCHEMA_VERSION,
        workspace: WorkspaceSpec {
            root: workspace_root_str,
            members: workspace_members,
        },
        crates,
        root_crate,
        workspace_members: workspace_member_keys,
        flake_metadata,
        // Single-target emission: target_resolves is None.
        // generate_multi_target populates it; this single-target path
        // leaves it absent so substrate falls back to per-crate edges.
        target_resolves: None,
        cargo_lock_sha256: hash_cargo_lock(root),
    })
}

/// Compute the SHA-256 hex digest of the workspace's Cargo.lock.
/// Returns `None` when the lockfile doesn't exist (rare — `cargo
/// metadata` would have already failed) or can't be read. Embedded
/// in the spec as a content-addressed cache key consumed by BOTH
/// the Rust fast-path (`gen build --if-stale`, `gen check-spec`)
/// AND the substrate Nix-side IFD gate (`builtins.hashFile "sha256"`
/// in `lockfile-builder.nix`). SHA-256 specifically because nix
/// has built-in hashFile but no BLAKE3 surface; the perf cost is
/// rounding error on <500 KB lockfiles.
fn hash_cargo_lock(root: &Path) -> Option<String> {
    use sha2::{Digest, Sha256};
    let lock_path = root.join("Cargo.lock");
    let bytes = std::fs::read(&lock_path).ok()?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Some(format!("{:x}", hasher.finalize()))
}

/// Parse `owner/repo` out of a GitHub-style URL. Accepts both `.git`
/// and bare forms. Returns None for non-canonical URLs so the consumer
/// must override explicitly rather than silently emit a wrong slug.
fn parse_owner_repo(url: &str) -> Option<String> {
    let stripped = url.trim_end_matches(".git");
    // Accept https://github.com/owner/name, git@github.com:owner/name,
    // ssh://git@github.com/owner/name. Reject anything else.
    let body = stripped
        .strip_prefix("https://github.com/")
        .or_else(|| stripped.strip_prefix("git@github.com:"))
        .or_else(|| stripped.strip_prefix("ssh://git@github.com/"))?;
    let mut parts = body.split('/');
    let owner = parts.next()?;
    let name = parts.next()?;
    if owner.is_empty() || name.is_empty() {
        return None;
    }
    Some(format!("{owner}/{name}"))
}

// Legacy shell-out `prefetch_git_sha256` removed at 2026-05-29 —
// replaced by the pure-Rust `git_prefetcher::GixPrefetcher` (gix +
// nix-nar streaming NAR-sha256). See `theory/RUST-NATIVE-PREFETCH.md`.

pub fn generate_and_write(root: &Path) -> Result<std::path::PathBuf> {
    generate_for_target_and_write(root, host_target_triple())
}

/// Typed spec freshness — read-only check that compares the
/// committed `Cargo.build-spec.json`'s `cargo_lock_hash` against
/// the current `Cargo.lock` content's BLAKE3 digest. No subprocess,
/// no spec regeneration; cheap enough to run per-repo across the
/// entire fleet in a tight loop.
///
/// Consumers (fleet's pre-rebuild sweep, CI invariant checks,
/// operator `gen check` diagnostics) use the typed `Freshness`
/// variants to make decisions: `Fresh` means skip regeneration,
/// `Drifted` / `Missing` / `UnhashedSpec` mean a regen is needed.
///
/// **Typed-dispatcher catalog member** (`gen.cargo.freshness`). The
/// 14th consumer class in gen-platform's fleet-wide catalog, joining
/// the per-ecosystem crate-quirks, cofre.backend-kind,
/// shigoto.retry-outcome, etc. Adding a Freshness variant
/// mechanically bumps `variant_count` in the fleet-catalog-coverage
/// test — the substrate enforces no silent drift.
#[gen_macros::fsm(label = "gen.cargo.freshness")]
pub enum Freshness {
    /// `Cargo.lock` hashes match the spec's stored hash — spec is
    /// byte-equal to what a regen would produce. Skip regeneration.
    Fresh {
        spec_hash: String,
        lock_hash: String,
    },
    /// Spec exists + has a stored hash, but it doesn't match the
    /// current `Cargo.lock`. Regen needed.
    Drifted {
        spec_hash: String,
        lock_hash: String,
    },
    /// Spec exists but predates the hash field (schema < 6). Treated
    /// as drift — regen to upgrade.
    UnhashedSpec { lock_hash: String },
    /// `Cargo.build-spec.json` doesn't exist. First-time emission
    /// needed.
    MissingSpec { lock_hash: String },
    /// `Cargo.lock` doesn't exist — the workspace isn't buildable
    /// in its current state. Operator-actionable; not a regen
    /// candidate.
    MissingLock,
}

impl Freshness {
    // `is_fresh()` / `is_drifted()` / `is_unhashed_spec()` /
    // `is_missing_spec()` / `is_missing_lock()` are emitted by the
    // `gen_macros::IsVariant` derive — no hand-rolled body needed.

    /// True iff `gen build` would do meaningful work.
    #[must_use]
    pub fn needs_regen(&self) -> bool {
        matches!(
            self,
            Freshness::Drifted { .. }
                | Freshness::UnhashedSpec { .. }
                | Freshness::MissingSpec { .. }
        )
    }

    /// Operator-facing one-line summary.
    #[must_use]
    pub fn summary(&self) -> &'static str {
        match self {
            Freshness::Fresh { .. } => "fresh",
            Freshness::Drifted { .. } => "drifted",
            Freshness::UnhashedSpec { .. } => "unhashed-spec",
            Freshness::MissingSpec { .. } => "missing-spec",
            Freshness::MissingLock => "missing-lock",
        }
    }
}

/// Minimal spec-header view used by `check_freshness`. Decoupled
/// from the full `BuildSpec` so an old or in-flight spec whose
/// nested shape has drifted (a new required field on a nested
/// `CrateSpec`, an old schema_version, etc.) still yields a useful
/// `Freshness` reading. The only fields we consult are `version`
/// (reserved for future "stale schema" classification) and
/// `cargo_lock_sha256`. Any failure to parse even this minimal
/// shape is treated as `MissingSpec`.
///
/// Backward-compat: also reads `cargo_lock_hash` (the v6 BLAKE3
/// field) — present-but-mismatched on a v7 fresh-check signals the
/// repo's spec needs regeneration to gain the SHA-256 field that
/// substrate's IFD gate consults.
#[derive(Deserialize)]
struct SpecHeader {
    #[serde(default)]
    #[allow(dead_code)] // reserved for future schema-staleness check
    version: Option<u32>,
    #[serde(default)]
    cargo_lock_sha256: Option<String>,
}

/// Compute the spec's freshness without touching the spec or
/// invoking cargo. Pure file I/O + two SHA-256 hashes.
#[must_use]
pub fn check_freshness(root: &Path) -> Freshness {
    let lock_hash = match hash_cargo_lock(root) {
        Some(h) => h,
        None => return Freshness::MissingLock,
    };
    let spec_path = root.join("Cargo.build-spec.json");
    let spec_bytes = match std::fs::read(&spec_path) {
        Ok(b) => b,
        Err(_) => return Freshness::MissingSpec { lock_hash },
    };
    // Permissive parse: only the two fields we actually need.
    // Any other field's evolution must not block the freshness
    // signal — the regen below will overwrite anyway.
    let header: SpecHeader = match serde_json::from_slice(&spec_bytes) {
        Ok(h) => h,
        Err(_) => return Freshness::MissingSpec { lock_hash },
    };
    match header.cargo_lock_sha256 {
        None => Freshness::UnhashedSpec { lock_hash },
        Some(spec_hash) if spec_hash == lock_hash => {
            Freshness::Fresh { spec_hash, lock_hash }
        }
        Some(spec_hash) => Freshness::Drifted { spec_hash, lock_hash },
    }
}

/// Idempotent variant of `generate_and_write` — fast-path no-op
/// when the spec is already fresh. Returns the `Freshness` outcome
/// alongside the spec path so callers can log the decision.
pub fn generate_and_write_if_stale(root: &Path) -> Result<(Freshness, std::path::PathBuf)> {
    let freshness = check_freshness(root);
    let spec_path = root.join("Cargo.build-spec.json");
    if freshness.is_fresh() {
        return Ok((freshness, spec_path));
    }
    let written = generate_multi_target_and_write(root)?;
    let post = check_freshness(root);
    Ok((post, written))
}

/// Multi-target emission: write a spec that covers every fleet target
/// (FLEET_TARGETS). One committed spec, every target's resolves
/// available — gen-bootstrap chicken-and-egg permanently resolved.
pub fn generate_multi_target_and_write(root: &Path) -> Result<std::path::PathBuf> {
    let mut spec = generate_multi_target(root)?;
    // Canonicalize BEFORE both the full-spec write AND the delta distill so
    // both artifacts are byte-identical across build platforms.
    spec.canonicalize();
    if let Err(violations) = crate::invariants::assert_well_formed(&spec) {
        return Err(CargoError::Io {
            path: root.to_path_buf(),
            source: std::io::Error::new(
                std::io::ErrorKind::Other,
                format!(
                    "gen build (multi-target): spec violates algorithmic invariants ({} issues):\n{}",
                    violations.len(),
                    serde_json::to_string_pretty(&violations).unwrap_or_default()
                ),
            ),
        });
    }
    let out = root.join("Cargo.build-spec.json");
    let body = serde_json::to_string_pretty(&spec).map_err(|e| CargoError::Io {
        path: out.clone(),
        source: std::io::Error::new(std::io::ErrorKind::Other, e.to_string()),
    })?;
    std::fs::write(&out, body).map_err(|source| CargoError::Io {
        path: out.clone(),
        source,
    })?;
    // Additive: emit the slim `Cargo.gen.lock` delta alongside the full
    // spec. Multi-target always carries `target_resolves`, so the delta is
    // always derivable here. A failed emit fails the build — never skipped.
    crate::gen_delta::write_gen_delta(root, &spec).map_err(|e| CargoError::Io {
        path: root.join("Cargo.gen.lock"),
        source: std::io::Error::new(std::io::ErrorKind::Other, e.to_string()),
    })?;
    prune_and_log(root);
    Ok(out)
}

/// Per-target variant of generate_and_write. Used by substrate's
/// mkBuildSpec IFD when constructing per-platform specs for the I4
/// invariant (cfg-conditional dep filtering); also the canonical
/// fleet-CI entrypoint for cross-build spec emission.
pub fn generate_for_target_and_write(root: &Path, target: &str) -> Result<std::path::PathBuf> {
    let mut spec = generate_for_target(root, target)?;
    spec.canonicalize();
    // Algorithmic guarantee: every emitted spec satisfies the
    // substrate-side invariants. Violations surface as typed errors
    // before the file lands on disk — operators never see a downstream
    // Nix build fail from an invariant gen-cargo could have caught.
    if let Err(violations) = crate::invariants::assert_well_formed(&spec) {
        return Err(CargoError::Io {
            path: root.to_path_buf(),
            source: std::io::Error::new(
                std::io::ErrorKind::Other,
                format!(
                    "gen lock-build: spec violates algorithmic invariants ({} issues):\n{}",
                    violations.len(),
                    serde_json::to_string_pretty(&violations).unwrap_or_default()
                ),
            ),
        });
    }
    let out = root.join("Cargo.build-spec.json");
    let body = serde_json::to_string_pretty(&spec).map_err(|e| CargoError::Io {
        path: out.clone(),
        source: std::io::Error::new(std::io::ErrorKind::Other, e.to_string()),
    })?;
    std::fs::write(&out, body).map_err(|source| CargoError::Io {
        path: out.clone(),
        source,
    })?;
    prune_and_log(root);
    Ok(out)
}

// ─── DETERMINISM CANONICALIZATION ──────────────────────────────────────
//
// CONTRACT FOR EVERY gen ADAPTER (cargo today; bundler/gomod/npm/pip/ansible/
// helm when they leave stub state — theory/ECOSYSTEM-INTAKE.md): a committed
// build-spec MUST be byte-identical regardless of the build host. Any adapter
// whose resolver emits maps/Vecs in dependency-resolution order owes the same
// two-boundary canonicalization shipped here for cargo: (1) output-bearing
// keyed-by-string maps are `BTreeMap` (canonical by construction); (2) a
// `canonicalize()` stable-sorts every resolve-ordered Vec by a total content
// key, called before BOTH the full-spec write and the delta distill. Until an
// adapter does real platform-dependent resolution its IndexMaps are inert, so
// this is a forward contract, not a present bug — but a live adapter that skips
// it reintroduces the cross-platform-drift class this section eliminates.
//
// `gen build` resolves per target via `cargo metadata --filter-platform`;
// cargo's resolve-traversal + `meta.packages` iteration order is NOT a stable
// cross-platform contract (linux CI ≠ darwin), so a naively-emitted spec is
// byte-UNSTABLE across build hosts — which breaks the GEN TYPED-SPEC
// CONTRACT's "re-render → empty git diff" freshness check. Every serialized
// keyed map is canonical (sorted by key) BY CONSTRUCTION (BTreeMap); the
// resolve-ordered Vecs (dep edges, features, renames, binaries) are NOT
// auto-canonical, so `canonicalize` stable-sorts each by a TOTAL content key.
//
// Content-preserving by construction: stable `sort_by`, never dedup/filter —
// only a permutation of an unchanged multiset (SCHEMA_VERSION unchanged).
// Substrate is provably order-independent (rustc `--extern` is name-explicit;
// the crate set is `lib.unique`; deps dispatch by `package_key` lookup;
// renames match by version — verified in lockfile-builder.nix), so reordering
// changes no build semantics. Idempotent.

/// Total, content-derived ordering key for a resolved dependency edge.
fn edge_sort_key(d: &CrateDepSpec) -> (&str, &str, u8, Option<&str>) {
    let kind_rank = match d.kind {
        DepKind::Normal => 0,
        DepKind::Build => 1,
        DepKind::Dev => 2,
    };
    (
        d.package_key.as_str(),
        d.name.as_str(),
        kind_rank,
        d.target.as_deref(),
    )
}

/// Stable-sort an edge Vec into canonical order, and canonicalize each edge's
/// own feature list. Keeps ALL entries (a rare duplicate-extern-name edge with
/// distinct `package_key`/kind/target stays, disambiguated by the secondary key).
fn canonical_edges(v: &mut [CrateDepSpec]) {
    for d in v.iter_mut() {
        d.features.sort();
    }
    v.sort_by(|a, b| edge_sort_key(a).cmp(&edge_sort_key(b)));
}

/// Sort each rename record list by `(version, rename)` — the map itself is a
/// BTreeMap (already key-sorted).
fn canonical_renames(m: &mut BTreeMap<String, Vec<CrateRenameRecord>>) {
    for recs in m.values_mut() {
        recs.sort_by(|a, b| (&a.version, &a.rename).cmp(&(&b.version, &b.rename)));
    }
}

/// Canonicalize every `CrateTargetEdges` value's Vecs in a per-target map.
fn canonical_edges_map(m: &mut BTreeMap<String, CrateTargetEdges>) {
    for e in m.values_mut() {
        canonical_edges(&mut e.dependencies);
        canonical_edges(&mut e.runtime_dependencies);
        canonical_edges(&mut e.build_dependencies);
        e.features.sort();
    }
}

impl BuildSpec {
    /// Canonicalize all resolve-ordered Vec ordering so the serialized spec
    /// (and the delta distilled from it) is byte-identical across build
    /// platforms. Maps are already canonical (`BTreeMap`). Stable-sort,
    /// content-preserving, idempotent. Call once before serializing the full
    /// spec or distilling `Cargo.gen.lock`.
    pub fn canonicalize(&mut self) {
        for c in self.crates.values_mut() {
            canonical_edges(&mut c.dependencies);
            canonical_edges(&mut c.runtime_dependencies);
            canonical_edges(&mut c.build_dependencies);
            c.features.sort();
            c.binaries.sort_by(|a, b| a.name.cmp(&b.name));
            canonical_renames(&mut c.crate_renames);
            canonical_renames(&mut c.build_rust_crate_args.crate_renames);
            c.build_rust_crate_args.features.sort();
        }
        if let Some(tr) = self.target_resolves.as_mut() {
            canonical_edges_map(&mut tr.base);
            for ov in tr.targets.values_mut() {
                canonical_edges_map(&mut ov.overrides);
            }
        }
    }
}

/// Remove deprecated crate2nix-era sidecars that the typed build-spec
/// supersedes. Returns the removed path (for logging) or `None` when
/// absent.
///
/// `crate-hashes.json` mapped each git dependency to a base32 nar hash
/// for crate2nix. The gen pipeline replaced it: git-source hashes now
/// live in `Cargo.build-spec.json` (`source.sha256`, SRI, computed from
/// `Cargo.lock`), and substrate's `lockfile-builder` fetches git deps
/// from there. The file is never read on the gen path — substrate only
/// touches it in the legacy crate2nix fallback, which `rm`s and
/// regenerates it anyway — so a committed copy is pure drift that goes
/// stale on every dependency bump. Every spec write (`gen build`,
/// `lock-build`, `fleet-sweep`, and substrate's IFD regen) prunes it so
/// the cleanup is mechanical and new repos never accumulate it.
pub fn prune_deprecated_sidecars(root: &Path) -> std::io::Result<Option<std::path::PathBuf>> {
    let legacy = root.join("crate-hashes.json");
    if legacy.is_file() {
        std::fs::remove_file(&legacy)?;
        Ok(Some(legacy))
    } else {
        Ok(None)
    }
}

/// Prune deprecated sidecars and emit a one-line operator notice.
/// Called after each successful spec write. A removal failure (e.g. a
/// read-only tree) is a warning, never fatal — the fresh spec has
/// already landed and is what every consumer reads.
fn prune_and_log(root: &Path) {
    match prune_deprecated_sidecars(root) {
        Ok(Some(path)) => eprintln!(
            "gen: pruned deprecated {} (superseded by Cargo.build-spec.json source hashes)",
            path.display()
        ),
        Ok(None) => {}
        Err(e) => eprintln!(
            "gen: warning — could not prune deprecated crate-hashes.json: {e}"
        ),
    }
}

/// Extract the resolved git rev from a Cargo `git+<url>?<query>#<rev>` source
/// string. Returns None for non-git sources or a `#`-less string. The rev is
/// the disambiguator in the `name-version-rev` crate key
/// (TOOLCHAIN-FRESHNESS §X.4b.b): two git revs of one crate version must key
/// distinctly so BOTH source trees survive in the BuildSpec instead of
/// colliding into one (the old behaviour silently dropped a rev).
fn git_rev_of_source(src: &str) -> Option<String> {
    if !src.starts_with("git+") {
        return None;
    }
    src.rsplit_once('#').map(|(_, rev)| rev.to_string())
}

/// Decide whether an OPTIONAL dependency edge is actually activated for a
/// crate C, given C's cargo-resolved (already-closed) feature set AND C's raw
/// manifest feature table (with `dep:` / `name/feat` / `name?/feat` strings
/// intact).
///
/// cargo's `resolve.nodes[].deps` is *over-inclusive* for optional deps — it
/// enumerates every optional dependency a package declares regardless of
/// whether the current feature set turns it on. The activated graph (what
/// `cargo tree` shows, what the built binary links) contains only the
/// optional deps the active feature set actually turns on. We must reconstruct
/// that activation decision from the FULL matrix below — keying only on the
/// implicit feature is WRONG because cargo SUPPRESSES the implicit feature
/// whenever ANY feature anywhere uses the `dep:<name>` syntax for that dep
/// (then the dep is named directly, and no synthetic feature exists).
///
/// An optional dep `name` of C is ACTIVATED (emit the edge) iff, scanning C's
/// **active** feature definitions (the resolved closure ∩ the manifest table),
/// any of:
///   1. `dep:<name>` appears (explicit activation; NO implicit feature in this
///      mode — the deranged `serde = ["dep:serde_core"]` case);
///   2. a NON-weak `<name>/<subfeat>` appears (slash form also activates the
///      dep);
///   3. the bare implicit feature `<name>` is itself active (the legacy bare
///      case — only exists when NO `dep:` form is used for that dep anywhere).
/// It is NOT activated when `name` is referenced ONLY through the WEAK
/// `<name>?/<subfeat>` form (weak refs add the sub-feature only when the dep
/// is already active — they never activate it; the sqlx-sqlite bug:
/// `migrate = [.., "sqlx-sqlite?/migrate"]` must NOT pull in sqlx-sqlite).
///
/// All comparisons normalize `-`/`_` because cargo's `extern crate` edge names
/// are underscored while declared dep names + feature names keep the manifest
/// hyphenation. `active_features` is cargo's resolved feature list for C
/// (already the transitive closure); `feature_table` is C's raw
/// `Package.features` map.
fn optional_dep_activated(
    active_features: &[String],
    feature_table: &std::collections::BTreeMap<String, Vec<String>>,
    dep_name: &str,
) -> bool {
    let norm = |s: &str| s.replace('-', "_");
    let target = norm(dep_name);

    // (3) Bare implicit feature active — the legacy case where cargo
    // synthesized a `<name>` feature (only when no `dep:` form is used).
    if active_features.iter().any(|f| norm(f) == target) {
        return true;
    }

    // (1) + (2): scan the ACTIVE feature definitions for an explicit `dep:`
    // activation or a non-weak `<name>/…` reference. Distinguish the weak
    // `<name>?/…` form (which does NOT activate).
    let active: std::collections::BTreeSet<String> =
        active_features.iter().map(|f| norm(f)).collect();
    for (feat, refs) in feature_table {
        if !active.contains(&norm(feat)) {
            continue;
        }
        for r in refs {
            // `dep:<name>` — explicit activation.
            if let Some(activated) = r.strip_prefix("dep:") {
                if norm(activated) == target {
                    return true;
                }
                continue;
            }
            // `<lhs>/<subfeat>` — slash form. The dep is the LHS; a trailing
            // `?` on the LHS marks it weak (does NOT activate).
            if let Some((lhs, _subfeat)) = r.split_once('/') {
                if let Some(weak_lhs) = lhs.strip_suffix('?') {
                    // Weak ref — never activates the dep. Skip.
                    let _ = weak_lhs;
                    continue;
                }
                if norm(lhs) == target {
                    return true;
                }
            }
            // A bare `<feat>` reference here is a feature→feature edge cargo
            // already followed into the resolved closure (handled by (3));
            // it never names a `dep:`-only optional dep, so nothing to do.
        }
    }

    false
}

/// Pre-shape crateRenames into the exact attrset shape nixpkgs's
/// buildRustCrate expects. Keys are canonical published names; values
/// are lists of `{ version, rename }` records, one per consumer-side
/// rename. Nix consumes this verbatim — no further synthesis.
fn synthesize_crate_renames(
    runtime: &[CrateDepSpec],
    build: &[CrateDepSpec],
    by_id: &IndexMap<String, &cargo_metadata::Package>,
) -> BTreeMap<String, Vec<CrateRenameRecord>> {
    let mut out: BTreeMap<String, Vec<CrateRenameRecord>> = BTreeMap::new();
    for d in runtime.iter().chain(build.iter()) {
        // Parse canonical from package_key ("<name>-<version>").
        // We could carry the canonical name as a field too — current
        // scheme keeps the spec compact.
        let canonical_name = {
            // package_key encodes "<name>-<version>"; we can't just
            // split on '-' because crate names contain hyphens. Look
            // up by package_key suffix-matching against by_id.
            let pkg = by_id.values().find(|p| {
                let base = format!("{}-{}", p.name, p.version);
                // Git deps key by `name-version-rev` (X.4b.b) — match that, else
                // the rename for a git dep is silently dropped.
                let k = match p.source.as_ref().map(|s| s.to_string()) {
                    Some(s) => match git_rev_of_source(&s) {
                        Some(rev) if !rev.is_empty() => {
                            format!("{}-{}-{}", p.name, p.version, rev)
                        }
                        _ => base,
                    },
                    None => base,
                };
                k == d.package_key
            });
            match pkg {
                Some(p) => (p.name.to_string(), p.version.to_string()),
                None => continue,
            }
        };
        let canonical = canonical_name.0;
        let canonical_version = canonical_name.1;
        // Only emit when the local alias differs from canonical.
        if d.name == canonical {
            continue;
        }
        out.entry(canonical).or_default().push(CrateRenameRecord {
            version: canonical_version,
            rename: d.name.clone(),
        });
    }
    out
}

/// Strip `dir/` prefix from `path` and return the remainder. Used to
/// translate absolute build-script paths into relative-to-manifest
/// paths the substrate consumer can pass to buildRustCrate.
fn strip_dir_prefix(path: &str, dir: &str) -> Option<String> {
    let dir_trim = dir.trim_end_matches('/');
    let prefix = format!("{dir_trim}/");
    path.strip_prefix(&prefix).map(String::from)
}

/// Compute relative path FROM `from` TO `base`, returning a
/// possibly-`..`-escaping form. Used when the source path lives
/// OUTSIDE the workspace root — e.g., a path-dep declared as
/// `gen-platform = { path = "../gen/crates/gen-platform" }` from
/// `kura/`. Walks up `base` until `from` becomes prefixable, then
/// returns `(.. * N)/<remainder>`. Returns None if the two paths
/// have no common ancestor (e.g., different drives on Windows;
/// not a case the substrate cares about). Both inputs are display
/// paths.
fn relative_path_escaping(from: &str, base: &str) -> Option<String> {
    let from_components: Vec<&str> =
        from.trim_end_matches('/').split('/').filter(|c| !c.is_empty()).collect();
    let base_components: Vec<&str> =
        base.trim_end_matches('/').split('/').filter(|c| !c.is_empty()).collect();
    // Find common prefix length.
    let common: usize = from_components
        .iter()
        .zip(base_components.iter())
        .take_while(|(a, b)| a == b)
        .count();
    if common == 0 {
        return None;
    }
    let up_count = base_components.len() - common;
    let down: Vec<&str> = from_components[common..].to_vec();
    let mut parts: Vec<String> = std::iter::repeat_n("..".to_string(), up_count).collect();
    parts.extend(down.into_iter().map(String::from));
    if parts.is_empty() {
        return Some(".".to_string());
    }
    Some(parts.join("/"))
}

/// Compute relative path from `from` to `base`. Returns None if
/// `from` doesn't start with `base`. Both inputs are display paths.
fn pathdiff_relative(from: &str, base: &str) -> Option<String> {
    let base_trim = base.trim_end_matches('/');
    if from == base_trim {
        return Some(String::new());
    }
    let with_slash = format!("{base_trim}/");
    from.strip_prefix(&with_slash).map(String::from)
}

// Catalog registration (`gen.cargo.freshness`) emitted by the
// `#[gen_macros::fsm(label = "...")]` on the enum above. 14th
// consumer class — same algebra as cofre.backend-kind,
// shigoto.retry-outcome, kura.node-kind. The macro keeps registration
// in lockstep with the variant universe; silent drift impossible.

/// Why an OPTIONAL dependency edge survives (or is dropped from) the build
/// spec — the single emit/skip decision at the resolve-edge call site, which
/// was previously an opaque inline boolean
/// (`!target_gated && !optional_dep_activated(...)`).
///
/// EXHAUSTIVE over the four reachable outcomes; the sole non-emitting variant
/// is [`OptionalDepActivation::Unactivated`]. Behavior-preserving by
/// construction — [`classify_optional_dep`] reproduces the old boolean exactly.
///
/// NOTE (tier-honest): this enum NAMES the decision; it does not yet close the
/// known coverage gaps. [`OptionalDepActivation::TargetGated`] trusts the
/// platform match alone, even for a dep that is ALSO weak/feature-gated — over-
/// including rather than DROPPING a platform-essential dep (rustix's libc on
/// apple). That tradeoff is deliberate: dropping a target-cfg backend dep wedges
/// the build (`error[E0432]: unresolved import libc`), whereas over-including a
/// rare target+feature-gated optional is usually harmless. Tightening the
/// target+feature intersection is a future ONE-ARM change here (split
/// `TargetGated` into trusted / feature-checked), NOT a call-site rewrite.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OptionalDepActivation {
    /// The dep is non-optional. The `if optional` guard at the call site is
    /// never entered, so the edge is emitted unconditionally.
    NotOptional,
    /// Optional AND the matched manifest declaration is target-scoped
    /// (`declared.target.is_some()`). cargo's `--filter-platform` resolve
    /// already evaluated the `[target.'cfg(...)']` guard and listed the edge
    /// only when the platform matched, so it is trusted as activated and the
    /// feature filter is bypassed (rustix's `libc`/`libc_errno` on apple/bsd).
    TargetGated,
    /// Optional, NOT target-gated, AND an active feature names it via
    /// `dep:<name>`, a NON-weak `<name>/x` slash form, or the bare implicit
    /// `<name>` feature (see [`optional_dep_activated`]).
    FeatureActivated,
    /// Optional, NOT target-gated, AND no active feature activates it —
    /// referenced only via a weak `<name>?/x` form, or by no active feature at
    /// all (sqlx-sqlite via `migrate = [.., "sqlx-sqlite?/migrate"]`). The sole
    /// non-emitting outcome.
    Unactivated,
}

impl OptionalDepActivation {
    /// True iff this edge must be emitted into the build spec. False ONLY for
    /// [`OptionalDepActivation::Unactivated`].
    fn is_emitted(self) -> bool {
        !matches!(self, OptionalDepActivation::Unactivated)
    }
}

/// Classify whether a resolved dependency edge is emitted, folding optionality,
/// the target-cfg `--filter-platform` trust, and the feature-matrix filter
/// ([`optional_dep_activated`]) into ONE typed, total decision. This is the one
/// place the emit rule lives.
///
/// Byte-identical to the old inline logic, including precedence (the target-cfg
/// arm is checked BEFORE the feature filter, reproducing the old
/// `!target_gated && !optional_dep_activated(...)`):
///   - `!is_optional`                 → `NotOptional`     (emit)
///   - `is_target_gated`              → `TargetGated`     (emit)
///   - `optional_dep_activated(...)`  → `FeatureActivated`(emit)
///   - else                           → `Unactivated`    (drop)
///
/// Takes plain booleans (not `&cargo_metadata::Dependency`) so it is a pure
/// function, fully unit-testable for every variant; the call site derives
/// `is_optional` / `is_target_gated` from the matched declaration.
fn classify_optional_dep(
    is_optional: bool,
    is_target_gated: bool,
    consuming_feats: &[String],
    feature_table: &std::collections::BTreeMap<String, Vec<String>>,
    dep_name: &str,
) -> OptionalDepActivation {
    if !is_optional {
        return OptionalDepActivation::NotOptional;
    }
    if is_target_gated {
        return OptionalDepActivation::TargetGated;
    }
    if optional_dep_activated(consuming_feats, feature_table, dep_name) {
        OptionalDepActivation::FeatureActivated
    } else {
        OptionalDepActivation::Unactivated
    }
}

#[cfg(test)]
mod weak_dependency_tests {
    use super::{classify_optional_dep, optional_dep_activated, OptionalDepActivation};
    use std::collections::BTreeMap;

    /// Build a feature table from `(feat, [refs])` pairs.
    fn ftable(pairs: &[(&str, &[&str])]) -> BTreeMap<String, Vec<String>> {
        pairs
            .iter()
            .map(|(f, refs)| {
                (
                    (*f).to_string(),
                    refs.iter().map(|r| (*r).to_string()).collect(),
                )
            })
            .collect()
    }

    /// Verification matrix (CLOSED-LOOP MASS-SYNTHESIS Rule 1) for
    /// `classify_optional_dep`: every activation OUTCOME has a row, and the
    /// exhaustive `match` over `OptionalDepActivation` is the forcing function —
    /// adding a variant without a row here is a COMPILE error. Pins the
    /// behavior-preserving contract: the edge is dropped iff `Unactivated`.
    #[test]
    fn classify_optional_dep_emit_matrix() {
        use OptionalDepActivation::{FeatureActivated, NotOptional, TargetGated, Unactivated};
        let empty = BTreeMap::new();

        // NotOptional: `!is_optional` short-circuits before the target arm,
        // regardless of target-scope or features.
        assert_eq!(
            classify_optional_dep(false, false, &[], &empty, "anything"),
            NotOptional
        );
        assert_eq!(
            classify_optional_dep(false, true, &[], &empty, "anything"),
            NotOptional
        );

        // TargetGated: optional + target-scoped → trusted on the platform match,
        // feature filter bypassed (rustix -> libc/libc_errno on apple/bsd).
        assert_eq!(
            classify_optional_dep(true, true, &[], &empty, "libc"),
            TargetGated
        );

        // FeatureActivated: optional, not target, an active feature names it —
        // here the bare implicit feature (sqlx -> sqlx-postgres via
        // `postgres = ["sqlx-postgres"]`).
        assert_eq!(
            classify_optional_dep(true, false, &["sqlx-postgres".to_string()], &empty, "sqlx-postgres"),
            FeatureActivated
        );

        // Unactivated: optional, not target, no active feature (sqlx-sqlite when
        // only weak-referenced / its gating feature is off). Sole non-emit case.
        assert_eq!(
            classify_optional_dep(true, false, &[], &empty, "sqlx-sqlite"),
            Unactivated
        );

        // is_emitted() forcing function — exhaustive over EVERY variant; a new
        // variant added without a row fails to compile.
        for v in [NotOptional, TargetGated, FeatureActivated, Unactivated] {
            let expect = match v {
                NotOptional | TargetGated | FeatureActivated => true,
                Unactivated => false,
            };
            assert_eq!(v.is_emitted(), expect, "{v:?}");
        }
    }

    /// The sqlx case, distilled. The `sqlx` facade declares optional driver
    /// deps `sqlx-postgres`, `sqlx-sqlite`, `sqlx-mysql`. A consumer that
    /// enables `["postgres", "migrate"]` (where `postgres = ["sqlx-postgres"]`
    /// and `migrate = [.., "sqlx-sqlite?/migrate", "sqlx-mysql?/migrate"]`)
    /// activates ONLY sqlx-postgres. cargo's resolved feature list reflects
    /// that — `sqlx-postgres` is present, the weak-only drivers are not.
    ///
    /// `cargo tree -i sqlx-sqlite` → "nothing to print"; the build spec must
    /// agree: the sqlx → sqlx-sqlite edge is dropped, sqlx → sqlx-postgres
    /// kept.
    #[test]
    fn weak_only_optional_dep_is_not_activated() {
        // sqlx uses NO `dep:` form for its drivers, so cargo SYNTHESIZES the
        // `sqlx-postgres`/`sqlx-sqlite`/`sqlx-mysql` implicit features. Only
        // the activated one lands in the resolved set.
        let table = ftable(&[
            ("postgres", &["sqlx-postgres"]),
            (
                "migrate",
                &["sqlx-postgres?/migrate", "sqlx-sqlite?/migrate", "sqlx-mysql?/migrate"],
            ),
        ]);
        let resolved = vec![
            "migrate".to_string(),
            "postgres".to_string(),
            "sqlx-postgres".to_string(), // implicit feat → postgres driver ON
            "runtime-tokio".to_string(),
            "uuid".to_string(),
            "chrono".to_string(),
        ];

        // The activated optional driver is pulled in (bare implicit feature).
        assert!(
            optional_dep_activated(&resolved, &table, "sqlx-postgres"),
            "sqlx-postgres is enabled via `postgres = [\"sqlx-postgres\"]` — keep the edge"
        );

        // The weak-only optional drivers are NOT — they are referenced ONLY
        // through `sqlx-sqlite?/migrate` / `sqlx-mysql?/migrate`, which never
        // activate the dep. Their implicit feature is absent.
        assert!(
            !optional_dep_activated(&resolved, &table, "sqlx-sqlite"),
            "sqlx-sqlite is reached only via the weak `sqlx-sqlite?/migrate` edge — drop it"
        );
        assert!(
            !optional_dep_activated(&resolved, &table, "sqlx-mysql"),
            "sqlx-mysql is reached only via the weak `sqlx-mysql?/migrate` edge — drop it"
        );
    }

    /// THE REGRESSION CASE. `deranged 0.5.8` declares `serde_core` as an
    /// optional dep and activates it ONLY via `serde = ["dep:serde_core"]`.
    /// Because a `dep:` form is used, cargo SUPPRESSES the `serde_core`
    /// implicit feature — so the resolved set is `["default", "serde"]` with
    /// NO `serde_core`. The old implicit-feature-only predicate wrongly
    /// dropped the edge; the full-matrix rule must KEEP it.
    #[test]
    fn dep_colon_activation_with_no_implicit_feature() {
        let table = ftable(&[
            ("default", &["std"]),
            ("std", &[]),
            ("serde", &["dep:serde_core"]),
        ]);
        // No `serde_core` in the resolved set — cargo suppressed the implicit
        // feature because `dep:serde_core` is used.
        let resolved = vec!["default".to_string(), "serde".to_string(), "std".to_string()];
        assert!(
            optional_dep_activated(&resolved, &table, "serde_core"),
            "serde = [\"dep:serde_core\"] is active → serde_core IS activated; emit the edge"
        );
    }

    /// A `dep:<name>` reference that lives in an INACTIVE feature must NOT
    /// activate the dep — the scan is over ACTIVE feature definitions only.
    #[test]
    fn dep_colon_in_inactive_feature_does_not_activate() {
        let table = ftable(&[("serde", &["dep:serde_core"])]);
        // `serde` feature NOT active.
        let resolved = vec!["default".to_string()];
        assert!(
            !optional_dep_activated(&resolved, &table, "serde_core"),
            "the `serde` feature carrying `dep:serde_core` is not active — drop the edge"
        );
    }

    /// Edge-name vs feature-name normalization: cargo's `extern crate` edge
    /// name is `_`-joined while the implicit feature keeps the manifest
    /// hyphenation. The predicate must see through that so a genuinely
    /// activated dep isn't dropped on a spurious string mismatch.
    #[test]
    fn activation_match_is_hyphen_underscore_insensitive() {
        let table = BTreeMap::new();
        let resolved = vec!["sqlx_postgres".to_string()];
        assert!(optional_dep_activated(&resolved, &table, "sqlx-postgres"));
        let resolved = vec!["sqlx-postgres".to_string()];
        assert!(optional_dep_activated(&resolved, &table, "sqlx_postgres"));
    }

    /// A non-weak `dep/feature` (no `?`) DOES activate the dep. Tested two
    /// ways: (a) the legacy bare implicit feature in the resolved set, and
    /// (b) directly from the slash-form reference in an active feature even
    /// when cargo suppressed the implicit feature.
    #[test]
    fn non_weak_reference_activates_the_dep() {
        // (a) `some-feat = ["mydep/x"]` (non-weak) → mydep implicit feature ON.
        let table = ftable(&[("some-feat", &["mydep/x"])]);
        let resolved = vec!["some-feat".to_string(), "mydep".to_string()];
        assert!(
            optional_dep_activated(&resolved, &table, "mydep"),
            "a non-weak dep/feature edge activates the dep — keep it (bare implicit)"
        );

        // (b) Same, but cargo suppressed the implicit feature (a `dep:` form
        // exists for mydep elsewhere). The non-weak slash form still activates.
        let table = ftable(&[("some-feat", &["mydep/x"]), ("other", &["dep:mydep"])]);
        let resolved = vec!["some-feat".to_string()];
        assert!(
            optional_dep_activated(&resolved, &table, "mydep"),
            "non-weak `mydep/x` activates mydep even with the implicit feature suppressed"
        );
    }

    /// A WEAK `<name>?/x` reference in an active feature must NOT activate.
    #[test]
    fn weak_slash_reference_in_active_feature_does_not_activate() {
        let table = ftable(&[("migrate", &["sqlx-sqlite?/migrate"])]);
        let resolved = vec!["migrate".to_string()];
        assert!(
            !optional_dep_activated(&resolved, &table, "sqlx-sqlite"),
            "weak `sqlx-sqlite?/migrate` does not activate the dep — drop the edge"
        );
    }

    #[test]
    fn no_features_means_no_optional_activation() {
        assert!(!optional_dep_activated(&[], &BTreeMap::new(), "anything"));
    }

    /// THE RENAMED-OPTIONAL-DEP REGRESSION CASE (attohttpc → rustls).
    ///
    /// attohttpc declares its rustls dep RENAMED:
    /// ```toml
    /// [dependencies]
    /// rustls_opt_dep = { package = "rustls", optional = true }
    /// [features]
    /// tls-rustls = ["tls-rustls-webpki-roots"]
    /// __rustls = ["rustls-opt-dep"]
    /// rustls-opt-dep = ["dep:rustls-opt-dep"]
    /// ```
    /// cargo names the implicit feature AND the `dep:` form by the
    /// CONSUMER-SIDE ALIAS `rustls-opt-dep`, never the package name
    /// `rustls`. So the activation key MUST be the rename. Keying on the
    /// package name `rustls` returns false → the edge is dropped → the
    /// consumer's rustc fails with E0463 `can't find crate for
    /// rustls_opt_dep`.
    ///
    /// This asserts the contract `optional_dep_activated` must satisfy:
    /// the dep activates under its rename-aware name, and is NOT found
    /// under the package name.
    #[test]
    fn renamed_optional_dep_activates_under_rename_not_package_name() {
        // attohttpc's feature table (the relevant subset), with the
        // rename-keyed implicit feature + the indirection through
        // __rustls and tls-rustls.
        let table = ftable(&[
            ("default", &["compress", "tls-native"]),
            ("__rustls", &["rustls-opt-dep"]),
            ("rustls-opt-dep", &["dep:rustls-opt-dep"]),
            ("tls-rustls", &["tls-rustls-webpki-roots"]),
            ("tls-rustls-webpki-roots", &["__rustls", "webpki-roots"]),
            ("webpki-roots", &["dep:webpki-roots"]),
        ]);
        // cargo's resolved active feature set for attohttpc as pulled by
        // gaveta (via aws-creds → rust-s3, json + rustls path).
        let resolved = vec![
            "__rustls".to_string(),
            "json".to_string(),
            "rustls-opt-dep".to_string(),
            "tls-rustls".to_string(),
            "tls-rustls-webpki-roots".to_string(),
            "webpki-roots".to_string(),
        ];

        // The dep activates under its RENAME (the rename-aware name gen
        // now keys on — `local_name` from the resolve node / `declared.rename`).
        // `-`↔`_` normalization makes `rustls_opt_dep` and `rustls-opt-dep`
        // equivalent.
        assert!(
            optional_dep_activated(&resolved, &table, "rustls-opt-dep"),
            "rustls is activated via the rename `rustls-opt-dep` (implicit feature + dep:) — keep the edge"
        );
        assert!(
            optional_dep_activated(&resolved, &table, "rustls_opt_dep"),
            "the `_`-joined extern form of the rename must also match (normalization)"
        );

        // Keying on the PACKAGE name `rustls` is the bug — it must NOT
        // match, proving the activation key has to be rename-aware.
        assert!(
            !optional_dep_activated(&resolved, &table, "rustls"),
            "the package name `rustls` is never the activation key for a renamed optional dep — the old code's mistake"
        );
    }
}

#[cfg(test)]
mod path_helper_tests {
    use super::{pathdiff_relative, relative_path_escaping};

    #[test]
    fn pathdiff_relative_returns_none_when_path_escapes_base() {
        // Workspace at /Users/me/code/kura; external path-dep at
        // /Users/me/code/gen/crates/gen-platform — outside the workspace.
        assert_eq!(
            pathdiff_relative(
                "/Users/me/code/gen/crates/gen-platform",
                "/Users/me/code/kura"
            ),
            None,
            "pathdiff_relative cannot represent escapes — that's relative_path_escaping's job"
        );
    }

    /// Regression for the gen-platform-external-path-dep bug:
    /// `kura-run` consumed gen-platform via `path = "../gen/crates/gen-platform"`,
    /// gen-cargo's old code fell back to `relative_path = "."` because
    /// the helper couldn't escape via `..`. lockfile-builder then looked
    /// for source at `/kura/src/lib.rs` (nothing) and produced an empty
    /// drv with no rlib. The consumer failed with "extern location for
    /// gen_platform does not exist".
    #[test]
    fn relative_path_escaping_handles_external_path_dep() {
        assert_eq!(
            relative_path_escaping(
                "/Users/me/code/gen/crates/gen-platform",
                "/Users/me/code/kura"
            ),
            Some("../gen/crates/gen-platform".to_string())
        );
    }

    #[test]
    fn relative_path_escaping_handles_sibling_workspaces() {
        assert_eq!(
            relative_path_escaping("/a/b/c", "/a/d/e"),
            Some("../../b/c".to_string())
        );
    }

    #[test]
    fn relative_path_escaping_returns_none_for_disjoint_roots() {
        assert_eq!(relative_path_escaping("/a/b", "/x/y"), None);
    }

    #[test]
    fn relative_path_escaping_returns_dot_for_same_dir() {
        // Same path → ".". Different from pathdiff_relative which returns
        // empty-string in this case.
        assert_eq!(
            relative_path_escaping("/a/b", "/a/b"),
            Some(".".to_string())
        );
    }

    // ------------------------------------------------------------------
    // Freshness primitive — typed BLAKE3 lock-hash for sweep fast-path.
    // ------------------------------------------------------------------

    fn freshness_tmpdir() -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static C: AtomicU64 = AtomicU64::new(0);
        let n = C.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!(
            "gen-cargo-freshness-{}-{}",
            std::process::id(),
            n
        ));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn check_freshness_missing_lock() {
        let dir = freshness_tmpdir();
        assert_eq!(
            super::check_freshness(&dir).summary(),
            "missing-lock"
        );
    }

    #[test]
    fn prune_removes_deprecated_crate_hashes_json() {
        // gen treats crate-hashes.json (a crate2nix-era sidecar) as
        // deprecated: every spec write prunes it. The build-spec's
        // SRI source hashes supersede it.
        let dir = freshness_tmpdir();
        let legacy = dir.join("crate-hashes.json");
        std::fs::write(&legacy, b"{}").unwrap();
        // First prune removes it and reports the path.
        let removed = super::prune_deprecated_sidecars(&dir).unwrap();
        assert_eq!(removed.as_deref(), Some(legacy.as_path()));
        assert!(!legacy.exists(), "crate-hashes.json should be pruned");
        // Idempotent: a second prune on a clean tree is a no-op.
        assert!(super::prune_deprecated_sidecars(&dir).unwrap().is_none());
    }

    #[test]
    fn check_freshness_missing_spec() {
        let dir = freshness_tmpdir();
        std::fs::write(dir.join("Cargo.lock"), b"# lock").unwrap();
        assert_eq!(
            super::check_freshness(&dir).summary(),
            "missing-spec"
        );
    }

    #[test]
    fn check_freshness_unhashed_spec_old_schema() {
        // A v6 (BLAKE3 `cargo_lock_hash`) OR v5 (no hash) spec must
        // surface as `unhashed-spec` — the permissive `SpecHeader`
        // parse exists precisely so a schema-drift on any nested
        // field doesn't masquerade as "no spec at all." v6 specs are
        // unhashed under v7 because the field name changed from
        // `cargo_lock_hash` (BLAKE3) to `cargo_lock_sha256`.
        let dir = freshness_tmpdir();
        std::fs::write(dir.join("Cargo.lock"), b"# lock").unwrap();
        // Spec missing `cargo_lock_sha256` AND with nested fields
        // (workspace.crates) that the full BuildSpec parser would
        // reject — only the header view should drive the decision.
        std::fs::write(
            dir.join("Cargo.build-spec.json"),
            br#"{"version": 5, "workspace": {"crates": {"x": {"this_field_doesnt_exist": true}}}}"#,
        )
        .unwrap();
        assert_eq!(
            super::check_freshness(&dir).summary(),
            "unhashed-spec"
        );
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(bytes);
        format!("{:x}", h.finalize())
    }

    #[test]
    fn check_freshness_fresh_when_hashes_match() {
        let dir = freshness_tmpdir();
        let lock_body: &[u8] = b"# lock\nfresh test\n";
        std::fs::write(dir.join("Cargo.lock"), lock_body).unwrap();
        let hash = sha256_hex(lock_body);
        std::fs::write(
            dir.join("Cargo.build-spec.json"),
            format!(r#"{{"version": 7, "cargo_lock_sha256": "{hash}"}}"#).as_bytes(),
        )
        .unwrap();
        let f = super::check_freshness(&dir);
        assert_eq!(f.summary(), "fresh");
        assert!(f.is_fresh());
        assert!(!f.needs_regen());
    }

    #[test]
    fn check_freshness_drifted_when_hashes_differ() {
        let dir = freshness_tmpdir();
        let lock_body: &[u8] = b"# new lock contents\n";
        std::fs::write(dir.join("Cargo.lock"), lock_body).unwrap();
        std::fs::write(
            dir.join("Cargo.build-spec.json"),
            br#"{"version": 7, "cargo_lock_sha256": "deadbeef"}"#,
        )
        .unwrap();
        let f = super::check_freshness(&dir);
        assert_eq!(f.summary(), "drifted");
        assert!(!f.is_fresh());
        assert!(f.needs_regen());
    }

    #[test]
    fn check_freshness_unparseable_spec_treated_as_missing() {
        let dir = freshness_tmpdir();
        std::fs::write(dir.join("Cargo.lock"), b"# lock").unwrap();
        // Not even valid JSON.
        std::fs::write(
            dir.join("Cargo.build-spec.json"),
            b"this is not json at all",
        )
        .unwrap();
        assert_eq!(
            super::check_freshness(&dir).summary(),
            "missing-spec"
        );
    }

    // ------------------------------------------------------------------
    // Typed-dispatcher catalog membership for Freshness.
    // ------------------------------------------------------------------
    //
    // The forcing-function pair: every Freshness variant has a stable
    // `summary()` AND the derived `discriminant()`; they must agree
    // and they must cover all 5 variants. If a variant lands without
    // a summary() arm OR the derive drops, the substrate fleet-
    // catalog-coverage-test fails by construction — but the round-
    // trip here surfaces the drift in the gen-cargo unit tests
    // first.
    #[test]
    fn freshness_summary_matches_discriminant_across_all_variants() {
        use super::Freshness;
        let cases = [
            Freshness::Fresh { spec_hash: "a".into(), lock_hash: "a".into() },
            Freshness::Drifted { spec_hash: "a".into(), lock_hash: "b".into() },
            Freshness::UnhashedSpec { lock_hash: "c".into() },
            Freshness::MissingSpec { lock_hash: "d".into() },
            Freshness::MissingLock,
        ];
        for f in &cases {
            assert_eq!(
                f.summary(),
                f.discriminant(),
                "Freshness::{:?} — summary() and discriminant() must agree",
                f
            );
        }
    }

    #[test]
    fn freshness_is_variant_helpers_cover_every_arm() {
        use super::Freshness;
        let fresh = Freshness::Fresh { spec_hash: "x".into(), lock_hash: "x".into() };
        let drifted = Freshness::Drifted { spec_hash: "x".into(), lock_hash: "y".into() };
        let unhashed = Freshness::UnhashedSpec { lock_hash: "x".into() };
        let missing_spec = Freshness::MissingSpec { lock_hash: "x".into() };
        let missing_lock = Freshness::MissingLock;

        // Each helper recognizes its own variant, rejects every other.
        assert!(fresh.is_fresh() && !drifted.is_fresh() && !unhashed.is_fresh());
        assert!(drifted.is_drifted() && !fresh.is_drifted() && !missing_lock.is_drifted());
        assert!(unhashed.is_unhashed_spec() && !fresh.is_unhashed_spec());
        assert!(missing_spec.is_missing_spec() && !drifted.is_missing_spec());
        assert!(missing_lock.is_missing_lock() && !fresh.is_missing_lock());
    }

    #[test]
    fn freshness_registered_in_fleet_catalog() {
        // The substrate fleet-catalog-coverage-test asserts
        // `variant_count == 5` against the snapshot. This test is
        // the matching forcing function on the Rust side: if a
        // variant lands or drops, the count diverges from 5 AND
        // the substrate test fails — drift can't slip through.
        use gen_platform::TypedDispatcherTrait;
        assert_eq!(
            <super::Freshness as TypedDispatcherTrait>::variant_count(),
            5,
            "Freshness variant_count must stay in lockstep with \
             substrate/lib/build/shared/fleet-catalog-coverage-test.nix \
             (label = gen.cargo.freshness)"
        );
        // Variant kinds are the kebab-case discriminants used by
        // serde + the catalog snapshot. Exact set is part of the
        // catalog contract — adding / renaming a kind is a
        // catalog-level change that must roll through the snapshot.
        assert_eq!(
            <super::Freshness as TypedDispatcherTrait>::variant_kinds(),
            vec!["fresh", "drifted", "unhashed-spec", "missing-spec", "missing-lock"],
        );
    }
}
