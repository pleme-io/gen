//! `Go.build-spec.json` — the canonical typed build manifest for the
//! gomod ecosystem.
//!
//! Two shapes live here, dispatched by [`BuildSpec::renderer`]:
//!
//! - **v2 incremental** (this module's top-level types) — one
//!   content-addressed derivation node PER Go package, keyed by
//!   `<import-path>#<goos>-<goarch>[+tags]`. Editing one package's
//!   sources rebuilds only that node + its transitive dependents;
//!   internal/shared packages compile once and are reused across every
//!   binary in the monorepo. This is `rustc-per-crate`, in Go.
//! - **v1 coarse** ([`coarse`]) — the module-level `buildGoModule`
//!   kwargs shape, preserved verbatim so the existing coarse
//!   `renderer: coarse` substrate path (`go/lockfile-builder.nix`)
//!   keeps working unchanged.
//!
//! Mirrors gen-cargo's `build_spec.rs`: every OUTPUT-BEARING keyed map
//! is a [`BTreeMap`] so the serialized JSON key order is canonical
//! (lexicographic) BY CONSTRUCTION — the spec is byte-identical
//! regardless of the resolver's traversal order (linux CI vs darwin).
//! `IndexMap` is used only for working/never-serialized maps.
//!
//! The encoder that populates this shape is [`crate::interp::apply`]
//! (driven by `go list -deps -json` over a vendored tree — see
//! [`crate::golist`]); the substrate interpreter is
//! `substrate/lib/build/go/package-builder.nix`.

use std::collections::BTreeMap;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

/// Schema version — v2 = per-package incremental (this module).
/// v1 = module-level coarse (see [`coarse::SCHEMA_VERSION`]).
pub const SCHEMA_VERSION: u32 = 2;

// ─────────────────────────────────────────────────────────────────────
// v1 coarse shape (preserved). The module-level buildGoModule kwargs the
// existing `renderer: coarse` substrate path spreads verbatim.
// ─────────────────────────────────────────────────────────────────────
pub mod coarse {
    //! Module-level `buildGoModule` kwargs — the v1 coarse shape.
    //! nixpkgs reference: `pkgs/build-support/go/module.nix`.

    use indexmap::IndexMap;
    use serde::{Deserialize, Serialize};

    pub const SCHEMA_VERSION: u32 = 1;

    #[derive(Clone, Debug, Serialize, Deserialize, gen_macros::SpecShape)]
    #[spec(
        args = "PackageArgs",
        quirk = "crate::quirks::GomodQuirk",
        args_field = "args",
        root_field = "root_package",
        members_field = "workspace_members",
        crates_field = "packages"
    )]
    pub struct BuildSpec {
        pub version: u32,
        pub packages: IndexMap<String, PackageSpec>,
        pub root_package: String,
        pub workspace_members: Vec<String>,
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub struct PackageSpec {
        pub name: String,
        pub version: String,
        pub args: PackageArgs,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub quirks: Vec<crate::quirks::GomodQuirk>,
    }

    /// Pre-shaped `buildGoModule` kwargs. Field names match nixpkgs'
    /// builder signature (camelCase via serde rename) so substrate
    /// spreads verbatim.
    #[derive(Clone, Debug, Default, Serialize, Deserialize)]
    pub struct PackageArgs {
        pub pname: Option<String>,
        pub version: Option<String>,
        #[serde(rename = "vendorHash", skip_serializing_if = "Option::is_none")]
        pub vendor_hash: Option<String>,
        #[serde(rename = "proxyVendor", skip_serializing_if = "Option::is_none")]
        pub proxy_vendor: Option<bool>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        pub tags: Vec<String>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        pub ldflags: Vec<String>,
        #[serde(rename = "subPackages", skip_serializing_if = "Vec::is_empty")]
        pub sub_packages: Vec<String>,
        #[serde(rename = "doCheck", skip_serializing_if = "Option::is_none")]
        pub do_check: Option<bool>,
        #[serde(skip_serializing_if = "IndexMap::is_empty")]
        pub env: IndexMap<String, String>,
        #[serde(rename = "nativeBuildInputs", skip_serializing_if = "Vec::is_empty")]
        pub native_build_inputs: Vec<String>,
        #[serde(rename = "buildInputs", skip_serializing_if = "Vec::is_empty")]
        pub build_inputs: Vec<String>,
    }
}

// ─────────────────────────────────────────────────────────────────────
// Target tuple + node-key canonicalization.
// ─────────────────────────────────────────────────────────────────────

/// One build target: `(goos, goarch, tags)`. M1 populates exactly one
/// tuple per build; the compact resolve shape is future-proof for the
/// multi-target milestone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetTuple {
    pub goos: String,
    pub goarch: String,
    /// Build tags in effect (already applied to each node's `go_files`
    /// by the encoder — Go-I6). Sorted for canonical suffixes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

impl TargetTuple {
    #[must_use]
    pub fn new(goos: impl Into<String>, goarch: impl Into<String>, mut tags: Vec<String>) -> Self {
        tags.sort();
        tags.dedup();
        Self { goos: goos.into(), goarch: goarch.into(), tags }
    }

    /// The build host's tuple, in Go naming (macos→darwin,
    /// aarch64→arm64, x86_64→amd64). No tags. The M1 default when the
    /// caller supplies no explicit target.
    #[must_use]
    pub fn host() -> Self {
        let goos = match std::env::consts::OS {
            "macos" => "darwin",
            other => other, // linux, windows, … map 1:1
        };
        let goarch = rust_arch_to_go(std::env::consts::ARCH);
        Self::new(goos, goarch, Vec::new())
    }

    /// Best-effort map from a Rust target triple (`AdapterCtx.target`,
    /// e.g. `aarch64-apple-darwin` / `x86_64-unknown-linux-musl`) to a
    /// Go `(goos, goarch)` tuple. Returns `None` when the OS token isn't
    /// recognized so the caller can fall back to [`host`](Self::host).
    #[must_use]
    pub fn from_rust_triple(triple: &str) -> Option<Self> {
        let parts: Vec<&str> = triple.split('-').collect();
        let arch = parts.first()?;
        let goos = if triple.contains("darwin") || triple.contains("apple") {
            "darwin"
        } else if triple.contains("linux") {
            "linux"
        } else if triple.contains("windows") {
            "windows"
        } else {
            return None;
        };
        Some(Self::new(goos, rust_arch_to_go(arch), Vec::new()))
    }

    /// The `#<goos>-<goarch>[+tag,tag]` suffix appended to a node key.
    /// Tags are sorted so the suffix is canonical regardless of the
    /// order `go list` reported them.
    #[must_use]
    pub fn suffix(&self) -> String {
        if self.tags.is_empty() {
            format!("#{}-{}", self.goos, self.goarch)
        } else {
            format!("#{}-{}+{}", self.goos, self.goarch, self.tags.join(","))
        }
    }
}

/// Map a Rust arch token to Go's `GOARCH`. Unknown tokens pass through
/// unchanged (Go and Rust agree on many: `arm`, `riscv64`, `s390x`, …).
fn rust_arch_to_go(arch: &str) -> &str {
    match arch {
        "aarch64" => "arm64",
        "x86_64" => "amd64",
        "x86" | "i686" => "386",
        "powerpc64" => "ppc64le",
        other => other,
    }
}

/// Canonical node key for a package at a tuple. Std packages get a
/// `std/` prefix so they never collide with a module package that
/// happens to share an import path.
#[must_use]
pub fn node_key(import_path: &str, kind: PackageKind, tuple: &TargetTuple) -> String {
    let base = if kind.is_std() {
        format!("std/{import_path}")
    } else {
        import_path.to_string()
    };
    format!("{base}{}", tuple.suffix())
}

// ─────────────────────────────────────────────────────────────────────
// v2 incremental shape — the per-package build graph.
// ─────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize, gen_macros::SpecShape)]
#[spec(
    args = "GoPackageArgs",
    quirk = "crate::quirks::GomodQuirk",
    args_field = "args",
    root_field = "root_package",
    members_field = "workspace_members",
    crates_field = "packages"
)]
pub struct BuildSpec {
    pub version: u32,
    /// `Coarse` = the v1 buildGoModule path; `Incremental` = this
    /// per-package graph. The M1 encoder always emits `Incremental`.
    pub renderer: Renderer,
    pub module: ModuleSpec,

    /// EVERY build-graph node, keyed canonically (see [`node_key`]).
    /// `BTreeMap` ⇒ canonical JSON key order by construction.
    #[serde(default)]
    pub packages: BTreeMap<String, PackageSpec>,

    /// The primary buildable node key (a `package main`).
    pub root_package: String,
    /// Every buildable `main` node (akeyless: logan/gator/auth/… → many).
    #[serde(default)]
    pub workspace_members: Vec<String>,

    /// Compact per-target import graph. M1 populates exactly one target;
    /// the shape is future-proof for the multi-target milestone.
    /// Lossless: substrate reconstructs `base // overrides[tuple]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_resolves: Option<GoCompactTargetResolves>,

    /// SHA-256 of `go.sum` content at emit — the D2 freshness tie
    /// (mirror of `cargo_lock_sha256`; consumed by substrate's
    /// `go/lockfile-delta.nix`). Empty-string hash `e3b0c4…` when the
    /// module is dep-free.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub go_sum_sha256: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, gen_macros::IsVariant)]
#[serde(rename_all = "kebab-case")]
pub enum Renderer {
    Coarse,
    Incremental,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModuleSpec {
    /// go.mod `module` directive, e.g. `akeyless.io/akeyless-main-repo`.
    pub module_path: String,
    /// go.mod `go` directive, e.g. `"1.26"`.
    pub go_version: String,
    /// go.mod `toolchain` directive when pinned, e.g. `"go1.26.4"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub toolchain: Option<String>,
    /// True when go.mod declares any `require` edge (drives the coarse
    /// vendorHash decision on the fallback path).
    pub has_external_deps: bool,
    /// M1 ⇒ `Vendored`. `Proxy` is the M-proxy milestone.
    pub dep_mode: DepMode,
    /// Coarse/fallback only. The M1 incremental path never fetches
    /// (`-mod=vendor`, `GOPROXY=off`) ⇒ `None`.
    #[serde(rename = "vendorHash", default, skip_serializing_if = "Option::is_none")]
    pub vendor_hash: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, gen_macros::IsVariant)]
#[serde(rename_all = "kebab-case")]
pub enum DepMode {
    Vendored,
    Proxy,
}

/// ONE build-graph node = one Go package compiled for one target tuple.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PackageSpec {
    /// Full import path, e.g.
    /// `akeyless.io/akeyless-main-repo/go/src/microservices/auth`.
    pub import_path: String,
    /// `Std` | `Module` | `Main` (Cgo/Tool are deferred to M-cgo).
    pub kind: PackageKind,
    /// `Vendored { relative_path }` | `Std`.
    pub source: PackageSource,
    /// The build-tree placement of THIS node (Go-I2 seam). Workload +
    /// std nodes are `Target`; build-tool / cgo-host nodes are `Host`
    /// (deferred kinds). M1 emits `Target` for every node — the seam
    /// exists so M-cgo is a classifier-arm swap, not a rewrite.
    #[serde(default)]
    pub tree: BuildTree,
    /// The *resolved* file list — build constraints already applied by
    /// the encoder for this tuple (Go-I6). Relative to the node's
    /// source root. Excludes `_test.go`.
    pub go_files: Vec<String>,
    /// The tuple's build tags in effect (already applied to `go_files`;
    /// kept for the compile invocation + audit provenance).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub build_tags: Vec<String>,
    /// `//go:embed` — patterns + resolved files (drives `-embedcfg`).
    #[serde(default, skip_serializing_if = "EmbedSpec::is_empty")]
    pub embed: EmbedSpec,
    /// Import edges → other node keys in `packages`. The per-package
    /// DAG; the interpreter builds `importcfg` from these (Go-I1).
    #[serde(default)]
    pub imports: Vec<String>,
    /// Vendor import rewrite map (go list `ImportMap`) — source import
    /// path → actual package path, when they differ (vendored deps).
    /// Carried so the interpreter can emit `importmap` lines.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub import_map: BTreeMap<String, String>,
    /// Provenance — the owning module (own module vs a vendored dep).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module: Option<PackageModuleRef>,
    /// BLAKE3 over (sorted `go_files` ⧺ `embed.files`) content — the
    /// incremental cache key + determinism/drift tie. (Nix's derivation
    /// hash is the *actual* store boundary; this is the encoder-side
    /// content address for the delta path + `gen check-spec` drift.)
    pub source_hash: String,
    pub args: GoPackageArgs,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quirks: Vec<crate::quirks::GomodQuirk>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, gen_macros::IsVariant)]
#[serde(rename_all = "kebab-case")]
pub enum PackageKind {
    Std,
    Module,
    Main,
    // + Cgo, Tool at M-cgo.
}

/// Which build tree a node compiles into (Go-I2). Mirrors gen-cargo's
/// `BuildTree`. Workload + std → `Target`; build-tooling / cgo-host →
/// `Host`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, gen_macros::IsVariant)]
#[serde(rename_all = "kebab-case")]
pub enum BuildTree {
    /// Built for the workload's arch. Default — every M1 node.
    #[default]
    Target,
    /// Built for the build-machine's arch. Tool / cgo-host nodes (M-cgo).
    Host,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum PackageSource {
    /// In-tree package — a subdir of the one committed workspace `src`
    /// (a vendored dep under `vendor/…`, OR the module's own package
    /// under `go/src/…`). Honors in-tree `replace` (go list points
    /// `Dir` at the replacement for free).
    Vendored { relative_path: String },
    /// Std package — provided by the shared std derivation, never fetched.
    Std,
    // Proxy { module, version, zip_sha256 } → M-proxy.
}

impl PackageSource {
    #[must_use]
    pub fn is_std(&self) -> bool {
        matches!(self, PackageSource::Std)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbedSpec {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub patterns: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
}

impl EmbedSpec {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.patterns.is_empty() && self.files.is_empty()
    }
}

/// The owning Go module for a node — provenance only. `path` is the
/// module path; `version` is `Some` for a vendored dep, `None` for the
/// main module.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageModuleRef {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// Pre-shaped `go tool compile`/`link` kwargs — spread verbatim by the
/// substrate interpreter.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GoPackageArgs {
    /// Extra `go tool compile` flags.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gcflags: Vec<String>,
    /// `go tool link` flags — link nodes only (e.g. `-X main.version=…`,
    /// `-s -w`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ldflags: Vec<String>,
    /// GOOS/GOARCH/CGO_ENABLED for THIS node.
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub env: IndexMap<String, String>,
}

// ─────────────────────────────────────────────────────────────────────
// Compact per-target resolves (mirror of gen-cargo, Go edge payload).
// M1 populates exactly one tuple; future-proof for M-multitarget.
// ─────────────────────────────────────────────────────────────────────

/// Per-node import edges for one target tuple — the per-(tuple)-varying
/// data (Go's build constraints select a different `go_files`/`imports`
/// per tuple). Keyed by the same node key as `BuildSpec.packages`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoTargetEdges {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub imports: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub import_map: BTreeMap<String, String>,
}

/// The FULL in-memory per-tuple resolve (one complete node-edges map per
/// tuple). The serialized form is [`GoCompactTargetResolves`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoTargetResolve {
    #[serde(default)]
    pub packages: BTreeMap<String, GoTargetEdges>,
}

/// Compact serialized representation of per-target resolves — `base`
/// holds edges identical across every tuple (stored once); `targets`
/// holds only the per-tuple differences. Decode contract:
/// `packages(tuple) = base // targets[tuple].overrides`.
///
/// Lifted verbatim in shape/algorithm from gen-cargo's
/// `CompactTargetResolves`. (Sharing ONE generic implementation across
/// cargo + gomod is the M-multitarget `gen-types` lift — see the M1
/// build doc §7.)
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoCompactTargetResolves {
    #[serde(default)]
    pub base: BTreeMap<String, GoTargetEdges>,
    #[serde(default)]
    pub targets: BTreeMap<String, GoTargetOverrides>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoTargetOverrides {
    #[serde(default)]
    pub overrides: BTreeMap<String, GoTargetEdges>,
}

impl GoCompactTargetResolves {
    /// Split a full per-tuple resolve map into base + overrides. A node
    /// is universal+identical iff it appears in every tuple with a
    /// byte-identical edge set. Lossless: for every tuple `T`,
    /// `base // overrides[T]` reconstructs the original map.
    #[must_use]
    pub fn from_full(full: IndexMap<String, GoTargetResolve>) -> Self {
        let Some((_first, first_resolve)) = full.iter().next() else {
            return Self::default();
        };

        let mut base: BTreeMap<String, GoTargetEdges> = BTreeMap::new();
        for (key, first_edges) in &first_resolve.packages {
            let present_in_all = full
                .values()
                .all(|resolve| resolve.packages.get(key) == Some(first_edges));
            if present_in_all {
                base.insert(key.clone(), first_edges.clone());
            }
        }

        let mut targets: BTreeMap<String, GoTargetOverrides> = BTreeMap::new();
        for (tuple, resolve) in &full {
            let mut overrides: BTreeMap<String, GoTargetEdges> = BTreeMap::new();
            for (key, edges) in &resolve.packages {
                if !base.contains_key(key) {
                    overrides.insert(key.clone(), edges.clone());
                }
            }
            targets.insert(tuple.clone(), GoTargetOverrides { overrides });
        }

        Self { base, targets }
    }

    /// Reconstruct the full per-tuple resolve map: `base // overrides[T]`
    /// for every tuple. Inverse of [`from_full`](Self::from_full).
    #[must_use]
    pub fn expand(&self) -> IndexMap<String, GoTargetResolve> {
        let mut out: IndexMap<String, GoTargetResolve> = IndexMap::new();
        for (tuple, over) in &self.targets {
            let mut packages = self.base.clone();
            for (key, edges) in &over.overrides {
                packages.insert(key.clone(), edges.clone());
            }
            out.insert(tuple.clone(), GoTargetResolve { packages });
        }
        out
    }
}

// ─────────────────────────────────────────────────────────────────────
// Content addressing — source_hash (BLAKE3) + go_sum_sha256 (SHA-256).
// ─────────────────────────────────────────────────────────────────────

/// BLAKE3 content address over a node's resolved sources. Entries are
/// `(relative_path, content_bytes)`; the hash is over the sorted-by-path
/// list with length-prefixed path + content (the estante/CA discipline),
/// so identical source ⇒ identical hash ⇒ identical node (cross-consumer
/// / cross-binary dedup, Go-I8).
#[must_use]
pub fn source_hash(entries: &[(String, Vec<u8>)]) -> String {
    let mut sorted: Vec<&(String, Vec<u8>)> = entries.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let mut hasher = blake3::Hasher::new();
    for (path, bytes) in sorted {
        hasher.update(&(path.len() as u64).to_le_bytes());
        hasher.update(path.as_bytes());
        hasher.update(&(bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
    }
    hasher.finalize().to_hex().to_string()
}

/// Lowercase-hex SHA-256 of `go.sum` content — the D2 freshness tie
/// (Go-I7). Matches `builtins.hashFile "sha256"`. Empty content yields
/// the canonical empty-string hash `e3b0c442…`.
#[must_use]
pub fn go_sum_sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tuple_suffix_is_canonical_and_tag_sorted() {
        let t = TargetTuple::new("linux", "amd64", vec!["osusergo".into(), "netgo".into()]);
        // tags sorted → netgo,osusergo regardless of input order.
        assert_eq!(t.suffix(), "#linux-amd64+netgo,osusergo");
        let bare = TargetTuple::new("darwin", "arm64", vec![]);
        assert_eq!(bare.suffix(), "#darwin-arm64");
    }

    #[test]
    fn node_key_prefixes_std() {
        let t = TargetTuple::new("linux", "amd64", vec![]);
        assert_eq!(node_key("fmt", PackageKind::Std, &t), "std/fmt#linux-amd64");
        assert_eq!(
            node_key("example.com/x/cmd/a", PackageKind::Main, &t),
            "example.com/x/cmd/a#linux-amd64"
        );
    }

    #[test]
    fn source_hash_is_order_independent_and_content_sensitive() {
        let a = vec![("b.go".to_string(), b"two".to_vec()), ("a.go".to_string(), b"one".to_vec())];
        let b = vec![("a.go".to_string(), b"one".to_vec()), ("b.go".to_string(), b"two".to_vec())];
        assert_eq!(source_hash(&a), source_hash(&b), "path order must not change the hash");
        let c = vec![("a.go".to_string(), b"one!".to_vec()), ("b.go".to_string(), b"two".to_vec())];
        assert_ne!(source_hash(&a), source_hash(&c), "a one-byte edit must change the hash");
    }

    #[test]
    fn empty_go_sum_is_the_canonical_empty_hash() {
        assert_eq!(
            go_sum_sha256(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn compact_resolves_round_trip_lossless() {
        let mut pkgs = BTreeMap::new();
        pkgs.insert(
            "example.com/x/a#linux-amd64".to_string(),
            GoTargetEdges { imports: vec!["std/fmt#linux-amd64".into()], import_map: BTreeMap::new() },
        );
        let mut full: IndexMap<String, GoTargetResolve> = IndexMap::new();
        full.insert("#linux-amd64".to_string(), GoTargetResolve { packages: pkgs.clone() });
        let compact = GoCompactTargetResolves::from_full(full.clone());
        assert_eq!(compact.expand(), full, "from_full ∘ expand must be identity");
    }
}
