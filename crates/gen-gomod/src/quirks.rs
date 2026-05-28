//! Typed quirk registry for gomod. Implements
//! `gen_types::QuirkRegistry` via `#[derive(QuirkRegistry)]`.
//!
//! Each registered entry names an upstream gomod package that needs
//! a known-good build-time workaround. The substrate consumer's
//! `gomod-quirk-apply.nix` dispatches mechanically on the variant
//! tags. Adding a new entry: append to `registry()` below.

use serde::{Deserialize, Serialize};

/// Typed quirks for known third-party upstream gomod packages.
/// Each variant maps to a Nix dispatch arm in
/// `substrate/lib/build/gomod/quirk-apply.nix`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, gen_macros::TypedDispatcher, gen_macros::Discriminant, gen_macros::IsVariant)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum GomodQuirk {
    /// Force a vendor-hash override — used when the proxy-fetched
    /// vendor tree's hash changes (proxy upgrade, mirror drift).
    ForceVendorHash { hash: String },
    /// Append a build tag — equivalent to `-tags <tag>` at build time.
    BuildTag { tag: String },
    /// Inject an ldflag — typically used for `-X
    /// main.version=<x>` at build time.
    Ldflag { flag: String },
    /// Disable CGO for this package.
    CgoOff,
    /// Inject a `go.mod`/`go.sum`/source patch via
    /// `substituteInPlace`.
    SubstituteSource { file: String, from: String, to: String },
}

pub fn registry() -> Vec<(&'static str, Vec<GomodQuirk>)> {
    // Hand-curated list of upstream-package quirks. Empty by
    // default; populate as the adapter encounters real bugs.
    Vec::new()
}

#[derive(gen_macros::QuirkRegistry)]
#[quirks(enum_name = "GomodQuirk", registry_fn = "crate::quirks::registry")]
pub struct GomodQuirks;

// Fleet-wide dispatcher-catalog registration.
gen_platform::register_dispatcher!("gen.gomod.gomod-quirk", GomodQuirk);
