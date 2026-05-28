//! Typed quirk registry for ansible. Implements
//! `gen_types::QuirkRegistry` via `#[derive(QuirkRegistry)]`.
//!
//! Each registered entry names an upstream ansible package that needs
//! a known-good build-time workaround. The substrate consumer's
//! `ansible-quirk-apply.nix` dispatches mechanically on the variant
//! tags. Adding a new entry: append to `registry()` below.

use serde::{Deserialize, Serialize};

/// Typed quirks for known third-party upstream ansible packages.
/// Add variants as needed; remember to mirror with a Nix dispatch
/// arm in `substrate/lib/build/ansible/quirk-apply.nix`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum AnsibleQuirk {
    // TODO: add ecosystem-specific quirk variants. Example shape:
    // ForceFlag { flag: String },
}

pub fn registry() -> Vec<(&'static str, Vec<AnsibleQuirk>)> {
    // Hand-curated list of upstream-package quirks. Empty by
    // default; populate as the adapter encounters real bugs.
    Vec::new()
}

#[derive(gen_macros::QuirkRegistry)]
#[quirks(enum_name = "AnsibleQuirk", registry_fn = "crate::quirks::registry")]
pub struct AnsibleQuirks;
