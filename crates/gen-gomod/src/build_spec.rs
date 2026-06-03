//! Typed build spec for gomod — mirrors nixpkgs `buildGoModule`
//! kwargs so substrate's Go-side lockfile-builder spreads the args
//! verbatim into the builder.
//!
//! nixpkgs reference:
//! `pkgs/build-support/go/module.nix` (`buildGoModule`).

use std::path::Path;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::parse;

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
    /// True when the module declares ≥1 external `require` edge — i.e.
    /// it needs a computed `vendorHash`. Distinct from `args.proxy_vendor`
    /// (which gen leaves at the nixpkgs default `false`, because forcing
    /// proxyVendor would change the hashed tree shape). This is the
    /// "needs-vendorHash" signal the invariant + the vendor-prefetch
    /// step key on.
    #[serde(default, skip_serializing_if = "is_false")]
    pub has_external_deps: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quirks: Vec<crate::quirks::GomodQuirk>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// Pre-shaped `buildGoModule` kwargs. Field names match nixpkgs'
/// builder signature (camelCase via serde rename) so substrate
/// spreads verbatim.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PackageArgs {
    pub pname: Option<String>,
    pub version: Option<String>,
    /// FOD hash of the vendored dep tarball. `null` (Nix) =
    /// `vendor/` directory already in source; `Some("sha256-…")` =
    /// proxy-fetched.
    #[serde(rename = "vendorHash", skip_serializing_if = "Option::is_none")]
    pub vendor_hash: Option<String>,
    /// Run `go mod vendor` against the proxy (true) vs git tarballs
    /// (false, default).
    #[serde(rename = "proxyVendor", skip_serializing_if = "Option::is_none")]
    pub proxy_vendor: Option<bool>,
    /// Build tags forwarded to `go build -tags …`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// `go build -ldflags …` entries.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ldflags: Vec<String>,
    /// Subpackages to build — paths relative to the module root.
    /// Default (empty) builds everything under `./...`.
    #[serde(rename = "subPackages", skip_serializing_if = "Vec::is_empty")]
    pub sub_packages: Vec<String>,
    /// Disable check phase. nixpkgs default = true.
    #[serde(rename = "doCheck", skip_serializing_if = "Option::is_none")]
    pub do_check: Option<bool>,
    /// Environment variables — e.g. `CGO_ENABLED=0`, `GOOS=linux`.
    #[serde(skip_serializing_if = "IndexMap::is_empty")]
    pub env: IndexMap<String, String>,
    /// Build-time native deps (cgo headers, generators).
    #[serde(rename = "nativeBuildInputs", skip_serializing_if = "Vec::is_empty")]
    pub native_build_inputs: Vec<String>,
    /// Link-time deps.
    #[serde(rename = "buildInputs", skip_serializing_if = "Vec::is_empty")]
    pub build_inputs: Vec<String>,
}

/// Generate the typed [`BuildSpec`] for the Go module rooted at `root`.
///
/// Mirrors gen-cargo's `generate`: read the on-disk manifest (`go.mod`
/// + `go.sum`) and shape a complete `PackageArgs` for the substrate
/// `buildGoModule` consumer. The `vendorHash` is left `None` here — it
/// is computed separately by [`crate::vendor_prefetcher`] during the
/// `lock` verb (network-bearing) and merged in by [`generate_and_write`]
/// when a prefetcher is supplied. `build` (the hermetic verb) reads the
/// already-computed hash from the committed delta; `generate` itself
/// performs zero subprocesses, so it is safe to call inside the nix
/// sandbox.
///
/// Defaults applied IN RUST (so the Nix consumer is a pure spread):
/// - `pname` = the module path's leaf segment.
/// - `version` = `"0.0.0"` placeholder (Go modules don't carry their
///   own version in go.mod; the build version is supplied out-of-band
///   by the substrate consumer or via a `BuildTag`/`Ldflag` quirk).
/// - `tags` / `ldflags` = empty (the nixpkgs `buildGoModule` defaults).
/// - `proxyVendor` = left `None` (= nixpkgs default `false`). The
///   default `goModules` derivation hashes the `go mod vendor`
///   extracted-source tree; forcing `proxyVendor = true` would change
///   the tree shape AND the vendorHash, so gen never forces it. The
///   need-a-vendorHash signal is `has_external_deps` (the presence of
///   `require` edges), NOT `proxyVendor`.
pub fn generate(root: &Path) -> Result<BuildSpec> {
    let go_mod = parse::parse_go_mod(root)?;
    let _go_sum = parse::parse_go_sum(root);

    let pname = go_mod.pname();
    let key = if pname.is_empty() {
        "root".to_string()
    } else {
        pname.clone()
    };

    let args = PackageArgs {
        pname: Some(if pname.is_empty() {
            "root".to_string()
        } else {
            pname.clone()
        }),
        version: Some("0.0.0".to_string()),
        vendor_hash: None,
        // Leave proxyVendor at the nixpkgs default (false) — see the
        // doc comment above; forcing it would invalidate the vendorHash.
        proxy_vendor: None,
        tags: Vec::new(),
        ldflags: Vec::new(),
        sub_packages: Vec::new(),
        do_check: None,
        env: IndexMap::new(),
        native_build_inputs: Vec::new(),
        build_inputs: Vec::new(),
    };

    let package = PackageSpec {
        name: go_mod.module.clone(),
        version: "0.0.0".to_string(),
        args,
        has_external_deps: !go_mod.requires.is_empty(),
        quirks: Vec::new(),
    };

    let mut packages = IndexMap::new();
    packages.insert(key.clone(), package);

    Ok(BuildSpec {
        version: SCHEMA_VERSION,
        packages,
        root_package: key,
        workspace_members: vec![go_mod.module],
    })
}

/// Generate the [`BuildSpec`] and write `Go.build-spec.json` +
/// `Go.gen.lock` next to the module's `go.mod`. Returns the path of the
/// full build-spec.
///
/// This is the production entrypoint wired into the CLI. The vendor
/// hash is computed via the production [`crate::vendor_prefetcher`] when
/// the module has external deps; a leaf module (no deps) skips the
/// prefetch entirely (no network, no `go` subprocess).
///
/// Additive: emits both the full `Go.build-spec.json` (for parity with
/// `Cargo.build-spec.json`) AND the slim `Go.gen.lock` delta. A failed
/// delta emit fails the whole call — never silently skipped (mirrors
/// gen-cargo's `generate_multi_target_and_write`).
pub fn generate_and_write(root: &Path) -> Result<std::path::PathBuf> {
    let prefetcher = crate::vendor_prefetcher::default_prefetcher();
    generate_and_write_with(root, prefetcher.as_ref())
}

/// `generate_and_write` parameterized over the vendor prefetcher — the
/// hermetic-test seam (inject a `MockVendorPrefetcher`).
pub fn generate_and_write_with(
    root: &Path,
    prefetcher: &dyn crate::vendor_prefetcher::VendorPrefetcher,
) -> Result<std::path::PathBuf> {
    let mut spec = generate(root)?;

    // Compute + merge the vendorHash for every package that declared
    // external deps. A leaf module skips this and emits a hash-less
    // spec (nixpkgs `vendorHash = null`).
    for pkg in spec.packages.values_mut() {
        if pkg.has_external_deps {
            let hash = prefetcher.prefetch(root).map_err(|e| {
                crate::error::GomodError::PrefetchVendorHashFailed {
                    root: root.to_path_buf(),
                    reason: format!("{} ({e})", pkg.name),
                }
            })?;
            pkg.args.vendor_hash = Some(hash.sri);
        }
    }

    // Write the full build spec.
    let out = root.join("Go.build-spec.json");
    let body = serde_json::to_string_pretty(&spec).map_err(|e| {
        crate::error::GomodError::Other(format!("serialize Go.build-spec.json: {e}"))
    })?;
    std::fs::write(&out, body + "\n")
        .map_err(|e| crate::error::GomodError::Other(format!("write Go.build-spec.json: {e}")))?;

    // Additive: emit the slim Go.gen.lock delta alongside the full spec.
    crate::gen_delta::write_gen_delta(root, &spec)
        .map_err(|e| crate::error::GomodError::Other(format!("write Go.gen.lock: {e}")))?;

    Ok(out)
}

/// Typed spec freshness for gomod — the analogue of gen-cargo's
/// `Freshness`. Compares the committed `Go.gen.lock`'s `go_sum_sha256`
/// freshness tie against the current `go.sum`'s SHA-256. Pure file I/O,
/// no subprocess; cheap enough to run per-repo across the fleet.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum Freshness {
    /// `go.sum` hash matches the delta's stored tie — skip regeneration.
    Fresh { tie: String, go_sum: String },
    /// Delta exists but its tie doesn't match the current `go.sum`.
    Drifted { tie: String, go_sum: String },
    /// `Go.gen.lock` doesn't exist — first-time emission needed.
    MissingDelta { go_sum: String },
    /// `go.mod` doesn't exist — not a Go module workspace.
    MissingManifest,
}

impl Freshness {
    /// True iff `gen build` would do meaningful work.
    #[must_use]
    pub fn needs_regen(&self) -> bool {
        matches!(self, Freshness::Drifted { .. } | Freshness::MissingDelta { .. })
    }

    /// Operator-facing one-line summary.
    #[must_use]
    pub fn summary(&self) -> &'static str {
        match self {
            Freshness::Fresh { .. } => "fresh",
            Freshness::Drifted { .. } => "drifted",
            Freshness::MissingDelta { .. } => "missing-delta",
            Freshness::MissingManifest => "missing-manifest",
        }
    }
}

/// Minimal `Go.gen.lock` header view for the freshness check —
/// decoupled from the full delta so a schema-evolved delta still yields
/// a useful reading.
#[derive(serde::Deserialize)]
struct DeltaHeader {
    #[serde(default)]
    go_sum_sha256: Option<String>,
}

/// Compute the gomod spec's freshness without regenerating anything.
#[must_use]
pub fn check_freshness(root: &Path) -> Freshness {
    if !root.join("go.mod").exists() {
        return Freshness::MissingManifest;
    }
    // The freshness tie is over go.sum content (empty-string hash when
    // the module is dependency-free / has no go.sum).
    let go_sum = match std::fs::read(root.join("go.sum")) {
        Ok(bytes) => crate::gen_delta::sha256_hex(&bytes),
        Err(_) => crate::gen_delta::sha256_hex(b""),
    };
    use gen_types::GenDeltaArtifact as _;
    let delta_path = root.join(crate::gen_delta::GoGenDelta::FILENAME);
    let bytes = match std::fs::read(&delta_path) {
        Ok(b) => b,
        Err(_) => return Freshness::MissingDelta { go_sum },
    };
    let header: DeltaHeader = match serde_json::from_slice(&bytes) {
        Ok(h) => h,
        Err(_) => return Freshness::MissingDelta { go_sum },
    };
    match header.go_sum_sha256 {
        Some(tie) if tie == go_sum => Freshness::Fresh { tie, go_sum },
        Some(tie) => Freshness::Drifted { tie, go_sum },
        None => Freshness::MissingDelta { go_sum },
    }
}
