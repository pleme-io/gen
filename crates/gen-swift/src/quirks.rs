//! Typed quirk registry for swift. Implements
//! `gen_types::QuirkRegistry` via `#[derive(QuirkRegistry)]`.
//!
//! Each registered entry names an upstream swift package that needs
//! a known-good build-time workaround. The substrate consumer's
//! `swift-quirk-apply.nix` dispatches mechanically on the variant
//! tags. Adding a new entry: append to `registry()` below.

use serde::{Deserialize, Serialize};

/// Typed quirks for known third-party upstream swift packages.
/// Each variant maps to a Nix dispatch arm in
/// `substrate/lib/build/swift/quirk-apply.nix`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, gen_macros::TypedDispatcher)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum SwiftQuirk {
    /// Pin a specific Swift toolchain version.
    PinToolchain { version: String },
    /// Force a build configuration override per package (e.g.
    /// `debug` for diagnostic builds).
    ForceConfiguration { configuration: String },
    /// Inject a linker flag specific to this package.
    Ldflag { flag: String },
    /// Inject a `Package.swift` patch via `substituteInPlace`.
    SubstituteSource { file: String, from: String, to: String },
}

pub fn registry() -> Vec<(&'static str, Vec<SwiftQuirk>)> {
    // Hand-curated list of upstream-package quirks. Empty by
    // default; populate as the adapter encounters real bugs.
    Vec::new()
}

#[derive(gen_macros::QuirkRegistry)]
#[quirks(enum_name = "SwiftQuirk", registry_fn = "crate::quirks::registry")]
pub struct SwiftQuirks;
