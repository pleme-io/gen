//! Typed quirk registry for pip. Implements
//! `gen_types::QuirkRegistry` via `#[derive(QuirkRegistry)]`.
//!
//! Each registered entry names an upstream pip package that needs
//! a known-good build-time workaround. The substrate consumer's
//! `pip-quirk-apply.nix` dispatches mechanically on the variant
//! tags. Adding a new entry: append to `registry()` below.

use serde::{Deserialize, Serialize};

/// Typed quirks for known third-party upstream pip packages.
/// Add variants as needed; remember to mirror with a Nix dispatch
/// arm in `substrate/lib/build/pip/quirk-apply.nix`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum PipQuirk {
    // TODO: add ecosystem-specific quirk variants. Example shape:
    // ForceFlag { flag: String },
}

pub fn registry() -> Vec<(&'static str, Vec<PipQuirk>)> {
    // Hand-curated list of upstream-package quirks. Empty by
    // default; populate as the adapter encounters real bugs.
    Vec::new()
}

#[derive(gen_macros::QuirkRegistry)]
#[quirks(enum_name = "PipQuirk", registry_fn = "crate::quirks::registry")]
pub struct PipQuirks;
