//! Typed `CrateQuirk` registry — the canonical knowledge of
//! "which third-party upstream crates need which buildRustCrate
//! class-helper applied."
//!
//! This module owns BOTH the type (the `CrateQuirk` enum that the
//! substrate consumer mechanically dispatches on) AND the data (the
//! `REGISTRY` mapping crate names to the quirks they need). gen-cargo
//! emits per-crate `quirks: Vec<CrateQuirk>` into `Cargo.build-spec.json`;
//! substrate's lockfile-builder reads them and applies a class-helper
//! per variant.
//!
//! Adding a new quirk is one entry in `REGISTRY`. Adding a new
//! quirk class is one new `CrateQuirk` variant + one new dispatch
//! arm in `substrate/lib/build/rust/pleme-crate-overrides.nix`.
//!
//! See `theory/CRATE-QUIRKS.md` for the contract + the class-helper
//! dispatch table.
use serde::{Deserialize, Serialize};

/// Typed buildRustCrate quirks emitted into the spec per crate. The
/// substrate consumer's lockfile-builder dispatches each variant to a
/// class-helper function (forceCfg / foldNormalIntoBuild /
/// substituteSource) without per-crate Nix-attrset knowledge.
#[derive(
    Clone,
    Debug,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    gen_macros::TypedDispatcher,
    gen_macros::Discriminant,
    gen_macros::IsVariant,
)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum CrateQuirk {
    /// Force a `--cfg <name>` on the lib compile. Used for crates
    /// whose build.rs computes `cargo:rustc-cfg=<name>` via macros
    /// (e.g. `cfg_aliases!`) that nixpkgs' buildRustCrate build-script
    /// runner doesn't propagate to the lib compile. Maps to substrate's
    /// `forceCfg <name>` class-helper.
    ForceCfg { cfg: String },
    /// Fold the crate's normal dependencies into its buildDependencies
    /// so its build.rs can resolve them. Used for crates whose build.rs
    /// uses normal deps via `use X::…` or `extern crate X` AND the
    /// crate is itself sometimes pulled as a transitive build-dep
    /// (nixpkgs' buildRustCrate drops the inner build-dep tree in that
    /// path). Optional `extern_crate` injects `extern crate <name>;`
    /// into build.rs for edition-2021+ crates that omit it. Maps to
    /// substrate's `foldNormalIntoBuild { externCrate = …; }`.
    FoldNormalIntoBuild {
        // `Option` in a struct variant is NOT auto-defaulted by serde —
        // without `default`, a spec that omits `externCrate` fails the
        // full-`BuildSpec` round-trip with `missing field externCrate`.
        #[serde(default)]
        extern_crate: Option<String>,
    },
    /// Substitute a one-line patch into a source file. Used for
    /// upstream source bugs whose fix is a single-string replacement
    /// (e.g. openraft's type-inference ambiguity). Maps to substrate's
    /// `substituteSource <file> <from> <to>`.
    SubstituteSource {
        file: String,
        from: String,
        to: String,
    },
    /// Add extra `nativeBuildInputs` packages to the crate's
    /// buildRustCrate derivation. Used for crates whose build.rs shells
    /// out to a native toolchain binary (e.g. a compiler, `cmake`,
    /// `protoc`) that isn't part of the default buildRustCrate sandbox.
    /// Each `packages` entry must be a real top-level nixpkgs attribute
    /// name — the substrate consumer resolves it via `pkgs.${name}`.
    /// Maps to substrate's `nativeBuildInputs <names>` class-helper.
    ///
    /// Not a fit for crates that locate a *vendored* binary via
    /// `env!("CARGO_MANIFEST_DIR")` or their own build.rs's `OUT_DIR`
    /// (e.g. `protobuf-src`, `protoc-bin-vendored`) — that path is
    /// baked in at THEIR OWN compile time and doesn't survive past
    /// their own ephemeral Nix build sandbox once a *different* crate's
    /// build.rs calls the function later, in a separate derivation.
    /// Give the crate that actually needs the tool a real nixpkgs
    /// package on PATH instead (see the `vigy-rpc` registry entry).
    NativeBuildInputs { packages: Vec<String> },
}

/// The canonical registry. Sorted by crate name for stable diffs.
///
/// Every entry has a comment explaining WHY the quirk exists; the
/// pleme-io substrate doctrine is that residual quirks for upstream
/// third-party crates are a fact of life, but each one names the
/// upstream bug so future maintainers can audit when fixes land.
pub fn registry() -> Vec<(&'static str, Vec<CrateQuirk>)> {
    vec![
        // openraft 0.9.24's src/metrics/wait.rs:231 uses
        // `let got = …collect();` without a type annotation. When
        // rkyv lands in the same project's target/deps, rustc sees
        // two `BTreeSet: PartialEq` impls (`alloc` + rkyv's
        // `ArchivedBTreeSet`) and rejects inference.
        (
            "openraft",
            vec![CrateQuirk::SubstituteSource {
                file: "src/metrics/wait.rs".to_string(),
                from: "let got = m.membership_config.membership().voter_ids().collect();"
                    .to_string(),
                to: "let got: std::collections::BTreeSet<_> = m.membership_config.membership().voter_ids().collect();".to_string(),
            }],
        ),
        // clang-sys 1.8.x's build.rs writes `use glob::…` (edition
        // 2021) — `glob` is a build-dependency. When clang-sys is
        // pulled as a transitive build-dep (e.g. via
        // coreaudio-sys → bindgen), buildRustCrate drops the inner
        // build-dep tree, leaving target/buildDeps empty.
        (
            "clang-sys",
            vec![CrateQuirk::FoldNormalIntoBuild {
                extern_crate: Some("glob".to_string()),
            }],
        ),
        // mime_guess 2.0.x's build.rs does `extern crate unicase;`.
        // Same nested-build-dep drop as clang-sys.
        (
            "mime_guess",
            vec![CrateQuirk::FoldNormalIntoBuild { extern_crate: None }],
        ),
        // wgpu-hal + siblings use `cfg_aliases!` to compute BOTH
        // `supports_64bit_atomics` (from `target_has_atomic = "64"`) and
        // `supports_ptr_atomics` (from `target_has_atomic = "ptr"`). The
        // build script's `cargo:rustc-cfg=...` isn't propagated to the lib
        // compile by buildRustCrate, and gen does not model
        // `target_has_atomic`, so it drops BOTH cfgs AND the
        // `cfg(not(target_has_atomic = "ptr"))`-gated `portable-atomic-util`
        // dep — so wgpu-hal 27 takes the portable-atomic fallback arm and
        // fails E0432 (unresolved `portable_atomic_util`). Every host where
        // pleme-io GPU apps run (aarch64-darwin, x86_64-darwin/linux) has
        // native 64-bit AND pointer-width atomics, so forcing both is safe
        // and makes wgpu-hal take the `alloc::sync::Arc` arm (the fallback
        // portable-atomic dep is then never needed).
        (
            "wgpu-hal",
            vec![
                CrateQuirk::ForceCfg { cfg: "supports_64bit_atomics".to_string() },
                CrateQuirk::ForceCfg { cfg: "supports_ptr_atomics".to_string() },
            ],
        ),
        (
            "wgpu-core",
            vec![
                CrateQuirk::ForceCfg { cfg: "supports_64bit_atomics".to_string() },
                CrateQuirk::ForceCfg { cfg: "supports_ptr_atomics".to_string() },
            ],
        ),
        (
            "wgpu",
            vec![
                CrateQuirk::ForceCfg { cfg: "supports_64bit_atomics".to_string() },
                CrateQuirk::ForceCfg { cfg: "supports_ptr_atomics".to_string() },
            ],
        ),
        (
            "wgpu-types",
            vec![
                CrateQuirk::ForceCfg { cfg: "supports_64bit_atomics".to_string() },
                CrateQuirk::ForceCfg { cfg: "supports_ptr_atomics".to_string() },
            ],
        ),
        // pleme-io/vigy's vigy-rpc build.rs calls
        // `tonic_build::configure().compile_protos(...)`, which shells
        // out to a `protoc` binary. Vendored-protoc crates
        // (protobuf-src, protoc-bin-vendored) locate their bundled
        // binary via `env!("CARGO_MANIFEST_DIR")` / a build.rs-produced
        // OUT_DIR — a path baked in at THEIR OWN compile time that
        // doesn't survive past their own ephemeral Nix build sandbox,
        // so calling that function from a DIFFERENT crate's build.rs
        // (a separate derivation) always reports "protoc not found".
        // Give vigy-rpc's own build a real, nixpkgs-built `protoc` on
        // PATH instead (`pkgs.protobuf` is a normal nixpkgs derivation
        // with no baked-path problem) — tonic-build/prost-build search
        // PATH when `PROTOC` isn't set.
        (
            "vigy-rpc",
            vec![CrateQuirk::NativeBuildInputs { packages: vec!["protobuf".to_string()] }],
        ),
    ]
}

/// Lookup quirks for a crate name. Returns empty Vec if no quirks
/// registered.
pub fn for_crate(name: &str) -> Vec<CrateQuirk> {
    for (k, v) in registry() {
        if k == name {
            return v;
        }
    }
    Vec::new()
}

/// Every crate name that has at least one registered quirk. Used by
/// the invariants pass to detect drift between the registry and what
/// gen-cargo actually emitted into the spec.
pub fn registered_crate_names() -> Vec<&'static str> {
    registry().into_iter().map(|(k, _)| k).collect()
}

// Fleet-wide dispatcher-catalog registration. One line per adapter;
// the catalog gains the entry at link time via inventory.
gen_platform::register_dispatcher!("gen.cargo.crate-quirk", CrateQuirk);
