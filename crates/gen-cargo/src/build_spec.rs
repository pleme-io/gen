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
use serde::{Deserialize, Serialize};

use crate::error::{CargoError, Result};

/// Schema version — bump on breaking changes.
/// v2: + `flake_metadata`, `root_crate` is non-optional, git URLs
///     normalized (no `?branch=` suffix).
/// v3: + per-crate `build_rust_crate_args` (pre-shaped buildRustCrate
///     kwargs); + `links` + universal `preBuild`. Substrate's
///     lockfile-builder asserts on this version — older specs MUST
///     be regenerated via `gen build .` (no silent fallback).
pub const SCHEMA_VERSION: u32 = 5;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BuildSpec {
    pub version: u32,
    pub workspace: WorkspaceSpec,
    pub crates: IndexMap<String, CrateSpec>,
    /// The workspace's primary buildable crate's key in `crates`. Always
    /// populated — either a single-crate workspace's only member or the
    /// first workspace member by declaration order.
    pub root_crate: String,
    pub workspace_members: Vec<String>,
    /// Per-workspace-member metadata the Nix flake builder needs
    /// (toolName, repo, bin targets) — keyed by package name. Mirrors
    /// `cargo metadata`'s package.repository + targets without forcing
    /// Nix to re-parse Cargo.toml.
    pub flake_metadata: IndexMap<String, MemberFlakeMetadata>,
    /// Schema v5+: per-target resolved dep edges. When present,
    /// substrate's lockfile-builder reads dependencies from
    /// `target_resolves[currentTarget]` instead of the per-crate
    /// fields. Eliminates the gen-bootstrap chicken-and-egg —
    /// one committed spec serves every fleet target, no Nix-side
    /// cfg evaluation needed (cargo's resolver does cfg per-target,
    /// once per target, at spec-emission time).
    ///
    /// Old shape (top-level runtime_dependencies / build_dependencies
    /// on CrateSpec) is kept for backward compatibility. Substrate
    /// falls back when target_resolves is None.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_resolves: Option<IndexMap<String, TargetResolve>>,
    /// BLAKE3 hex (64 chars) of the workspace's Cargo.lock content at
    /// emit time. Drives the idempotence fast-path: when `gen build`
    /// runs and the current Cargo.lock's hash matches this value,
    /// the spec is byte-equal to what would be re-emitted and the
    /// write is skipped entirely (fleet-wide sweep cost becomes O(N)
    /// hash-checks rather than O(N) full regens).
    ///
    /// Missing on schema < 6 specs; `gen build` treats absent hash as
    /// "always re-emit" for backward compat. `gen check` uses the
    /// hash to compute typed `Freshness` per repo without writing.
    ///
    /// Schema bumped to 6 when this lands; substrate's
    /// lockfile-builder treats v5 and v6 identically (the field is
    /// purely a producer-side cache key).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cargo_lock_hash: Option<String>,
}

/// Per-target resolved dep edges for every crate in the workspace's
/// resolve graph. Substrate's lockfile-builder picks the entry that
/// matches the build's `targetTriple`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TargetResolve {
    /// Per-crate edge sets for this target. Keyed by the same
    /// `<name>-<version>` key as BuildSpec.crates.
    pub crates: IndexMap<String, CrateTargetEdges>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CrateTargetEdges {
    pub dependencies: Vec<CrateDepSpec>,
    pub runtime_dependencies: Vec<CrateDepSpec>,
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
    pub default_bin: Option<String>,
    /// `owner/name` parsed from `[package].repository`. None when the
    /// member doesn't declare a repository — consumer must override.
    pub repo: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkspaceSpec {
    pub root: String,
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
    pub build_script: Option<String>,
    /// `[package] links = "<symbol>"` declaration. Cargo passes this
    /// through as `CARGO_MANIFEST_LINKS` and ring's build.rs (and the
    /// whole `*-sys` family — bzip2-sys, libsqlite3-sys, openssl-sys,
    /// libz-sys, …) asserts on it. nixpkgs' buildRustCrate honors a
    /// `links` arg verbatim. Emitting this from spec avoids per-crate
    /// `links = ...` overrides in pleme-crate-overrides.nix.
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
    pub binaries: Vec<CrateBinSpec>,
    /// Library target when the crate exposes one. None for bin-only or
    /// build-script-only crates. Carries the crate-name override (e.g.
    /// `bzip2_sys` for bzip2-sys) AND the lib path override (e.g.
    /// `lib.rs` instead of the default `src/lib.rs`). Without this,
    /// buildRustCrate's auto-discovery misses crates that put their
    /// library at the root or rename it.
    pub lib_target: Option<LibTargetSpec>,
    /// All non-dev deps. Kept as a flat list for cross-tool consumers
    /// that need to walk the union (e.g. SBOM emitters).
    pub dependencies: Vec<CrateDepSpec>,
    /// Pre-split: deps with kind == "normal". The substrate consumer
    /// passes this list straight to buildRustCrate's `dependencies`
    /// arg — no Nix-side filtering.
    pub runtime_dependencies: Vec<CrateDepSpec>,
    /// Pre-split: deps with kind == "build". Threads into
    /// buildRustCrate's `buildDependencies`.
    pub build_dependencies: Vec<CrateDepSpec>,
    /// Pre-shaped crateRenames table — keyed by canonical
    /// published-name, valued as `[{ version, rename }]` records.
    /// Threads through to buildRustCrate's `crateRenames` arg
    /// verbatim. Nix doesn't do any synthesis here.
    pub crate_renames: IndexMap<String, Vec<CrateRenameRecord>>,
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

/// Pre-shaped attrset that the substrate consumer spreads directly
/// into `buildRustCrate { … }`. Field names match buildRustCrate's
/// `mkArgs` signature verbatim (camelCase). Optional fields are
/// emitted-iff-present so consumers see absence as "use default."
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BuildRustCrateArgs {
    #[serde(rename = "crateName", skip_serializing_if = "Option::is_none")]
    pub crate_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edition: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub features: Vec<String>,
    #[serde(rename = "crateRenames", skip_serializing_if = "IndexMap::is_empty")]
    pub crate_renames: IndexMap<String, Vec<CrateRenameRecord>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release: Option<bool>,
    #[serde(rename = "procMacro", skip_serializing_if = "Option::is_none")]
    pub proc_macro: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub build: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub links: Option<String>,
    #[serde(rename = "libName", skip_serializing_if = "Option::is_none")]
    pub lib_name: Option<String>,
    #[serde(rename = "libPath", skip_serializing_if = "Option::is_none")]
    pub lib_path: Option<String>,
    /// Pre-rustc shell snippet. Set for EVERY crate to export
    /// `CARGO_CRATE_NAME` (cargo's standard env, which buildRustCrate
    /// otherwise omits — rmcp's `env!("CARGO_CRATE_NAME")` and any
    /// future crate that reads the same env now Just Works without a
    /// per-crate override). Caller overrides should APPEND to this
    /// (not replace) to preserve the export.
    #[serde(rename = "preBuild", skip_serializing_if = "Option::is_none")]
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
        sha256: Option<String>,
    },
    /// Path source (workspace member) — relative to workspace root.
    Path { relative_path: String },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CrateDepSpec {
    /// Consumer-side identifier (`extern crate <name>`); equal to
    /// `package_key` after `/version` stripping when no rename.
    pub name: String,
    /// Key into BuildSpec.crates — uniquely identifies the resolved
    /// dep. Format: `<canonical-name>-<version>`.
    pub package_key: String,
    pub kind: DepKind,
    pub features: Vec<String>,
    pub uses_default_features: bool,
    pub optional: bool,
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

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
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
    // Per-target specs, indexed by target triple.
    let mut per_target: IndexMap<String, BuildSpec> = IndexMap::new();
    for target in FLEET_TARGETS {
        eprintln!("gen build: resolving for {}", target);
        let spec = generate_for_target(root, target)?;
        per_target.insert(target.to_string(), spec);
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
        let mut crates: IndexMap<String, CrateTargetEdges> = IndexMap::new();
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
    base.target_resolves = Some(resolves);

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
    let mut cmd = MetadataCommand::new();
    cmd.manifest_path(&manifest_path);
    if !target.is_empty() {
        cmd.other_options(vec!["--filter-platform".to_string(), target.to_string()]);
    }
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

    let mut crates: IndexMap<String, CrateSpec> = IndexMap::new();
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
        // Path-deps (members + external) get `Path { relative_path }`.
        // For members, relative_path is the subdir under the workspace
        // root. For external path-deps (e.g.
        // `gen-platform = { path = "../gen/crates/gen-platform" }`),
        // it's the workspace-relative path that escapes via `..`. The
        // consuming substrate (lockfile-builder) uses this to locate
        // the source dir when src = workspaceSrc; without an accurate
        // relative_path, buildRustCrate looks for `src/lib.rs` at the
        // workspace root and finds nothing.
        let source = if is_path_dep {
            let abs_dir = pkg.manifest_path.parent().map(|p| p.to_string()).unwrap_or_default();
            // For external path-deps that live OUTSIDE the workspace,
            // pathdiff_relative returns None (the prefix-strip fails).
            // We fall back to a `..`-relative path computed manually so
            // the consumer can find the source.
            let rel = pathdiff_relative(&abs_dir, &workspace_root_str)
                .or_else(|| relative_path_escaping(&abs_dir, &workspace_root_str))
                .unwrap_or_else(|| abs_dir.clone());
            CrateSource::Path {
                relative_path: if rel.is_empty() { ".".to_string() } else { rel },
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
            let package_key = format!("{}-{}", dep_pkg.name, dep_pkg.version);

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
                let declared = pkg.dependencies.iter().find(|d| {
                    let consumer_name = d.rename.clone().unwrap_or_else(|| d.name.clone());
                    &consumer_name == local_name && d.kind == graph_kind.kind
                });
                let (features, uses_default_features, optional) = match declared {
                    Some(d) => (
                        d.features.iter().map(String::from).collect(),
                        d.uses_default_features,
                        d.optional,
                    ),
                    None => (Vec::new(), true, false),
                };

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
        match crates.get(&key) {
            Some(prev) if prev.features.len() > new_entry.features.len() => {
                // existing is richer — keep it as-is
            }
            _ => {
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
    let mut flake_metadata: IndexMap<String, MemberFlakeMetadata> = IndexMap::new();
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
        flake_metadata.insert(
            pkg.name.to_string(),
            MemberFlakeMetadata { default_bin, repo },
        );
    }


    // Prefetch sha256 for every Git source so substrate's
    // lockfile-builder can dispatch pkgs.fetchgit with a fixed hash
    // (no IFD, no impure builtins.fetchGit). Sequential — could be
    // parallelized but git deps are typically few.
    for crate_spec in crates.values_mut() {
        if let CrateSource::Git { url, rev, sha256, .. } = &mut crate_spec.source {
            if sha256.is_none() && !url.is_empty() && !rev.is_empty() {
                match prefetch_git_sha256(url, rev) {
                    Ok(hash) => *sha256 = Some(hash),
                    Err(e) => {
                        eprintln!(
                            "gen lock-build: warn — failed to prefetch sha256 for {}#{}: {}",
                            url, rev, e
                        );
                    }
                }
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
        cargo_lock_hash: hash_cargo_lock(root),
    })
}

/// Compute the BLAKE3 hex digest of the workspace's Cargo.lock.
/// Returns `None` when the lockfile doesn't exist (rare — `cargo
/// metadata` would have already failed) or can't be read. Embedded
/// in the spec as a content-addressed cache key: `gen build`
/// re-emits only when this hash differs from the spec's stored
/// value; `gen check` returns a typed `Freshness` based on it.
fn hash_cargo_lock(root: &Path) -> Option<String> {
    let lock_path = root.join("Cargo.lock");
    let bytes = std::fs::read(&lock_path).ok()?;
    Some(blake3::hash(&bytes).to_hex().to_string())
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

/// Run `nix-prefetch-git --url URL --rev REV --quiet` and parse the
/// resulting JSON for the `sha256` field. Spawns a sub-process; needs
/// nix-prefetch-git on PATH (nix-shell -p nix-prefetch-git satisfies).
fn prefetch_git_sha256(url: &str, rev: &str) -> std::io::Result<String> {
    use std::process::Command;
    // Strip cargo's `?branch=...` / `?tag=...` / `?rev=...` query
    // suffix — nix-prefetch-git treats the URL literally and the `?`
    // form isn't a valid git URL.
    let clean_url = url.split('?').next().unwrap_or(url);
    // First try direct invocation; fall back to nix-shell wrapper.
    let direct = Command::new("nix-prefetch-git")
        .args(["--url", clean_url, "--rev", rev, "--quiet"])
        .output();
    let output = match direct {
        Ok(o) if o.status.success() => o,
        _ => Command::new("nix-shell")
            .args([
                "-p",
                "nix-prefetch-git",
                "--run",
                &format!("nix-prefetch-git --url {clean_url} --rev {rev} --quiet"),
            ])
            .output()?,
    };
    if !output.status.success() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!(
                "nix-prefetch-git failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let v: serde_json::Value = serde_json::from_str(&stdout)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("parse: {e}")))?;
    v["sha256"]
        .as_str()
        .map(String::from)
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::Other, "no sha256 field in output"))
}

pub fn generate_and_write(root: &Path) -> Result<std::path::PathBuf> {
    generate_for_target_and_write(root, host_target_triple())
}

/// Multi-target emission: write a spec that covers every fleet target
/// (FLEET_TARGETS). One committed spec, every target's resolves
/// available — gen-bootstrap chicken-and-egg permanently resolved.
pub fn generate_multi_target_and_write(root: &Path) -> Result<std::path::PathBuf> {
    let spec = generate_multi_target(root)?;
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
    Ok(out)
}

/// Per-target variant of generate_and_write. Used by substrate's
/// mkBuildSpec IFD when constructing per-platform specs for the I4
/// invariant (cfg-conditional dep filtering); also the canonical
/// fleet-CI entrypoint for cross-build spec emission.
pub fn generate_for_target_and_write(root: &Path, target: &str) -> Result<std::path::PathBuf> {
    let spec = generate_for_target(root, target)?;
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
    Ok(out)
}

/// Pre-shape crateRenames into the exact attrset shape nixpkgs's
/// buildRustCrate expects. Keys are canonical published names; values
/// are lists of `{ version, rename }` records, one per consumer-side
/// rename. Nix consumes this verbatim — no further synthesis.
fn synthesize_crate_renames(
    runtime: &[CrateDepSpec],
    build: &[CrateDepSpec],
    by_id: &IndexMap<String, &cargo_metadata::Package>,
) -> IndexMap<String, Vec<CrateRenameRecord>> {
    let mut out: IndexMap<String, Vec<CrateRenameRecord>> = IndexMap::new();
    for d in runtime.iter().chain(build.iter()) {
        // Parse canonical from package_key ("<name>-<version>").
        // We could carry the canonical name as a field too — current
        // scheme keeps the spec compact.
        let canonical_name = {
            // package_key encodes "<name>-<version>"; we can't just
            // split on '-' because crate names contain hyphens. Look
            // up by package_key suffix-matching against by_id.
            let pkg = by_id.values().find(|p| {
                let k = format!("{}-{}", p.name, p.version);
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
}
