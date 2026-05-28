//! Typed quirk registry for ansible. Implements
//! `gen_types::QuirkRegistry` via `#[derive(QuirkRegistry)]`.
//!
//! Each registered entry names an upstream ansible package that needs
//! a known-good build-time workaround. The substrate consumer's
//! `ansible-quirk-apply.nix` dispatches mechanically on the variant
//! tags. Adding a new entry: append to `registry()` below.

use serde::{Deserialize, Serialize};

/// Typed quirks for known third-party upstream ansible collections.
/// Each variant maps to a Nix dispatch arm in
/// `substrate/lib/build/ansible/quirk-apply.nix`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, gen_macros::TypedDispatcher)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum AnsibleQuirk {
    /// Drop a declared cross-collection dependency — used when a
    /// collection over-declares deps it doesn't actually consume.
    DropDependency { collection: String },
    /// Pin a cross-collection dep to a specific version override.
    PinDependency { collection: String, version: String },
    /// Add a path to `build_ignore` — exclude from the built tarball.
    BuildIgnore { path: String },
    /// Inject a YAML/JSON patch into a vendored playbook/role file.
    SubstituteSource { file: String, from: String, to: String },
}

pub fn registry() -> Vec<(&'static str, Vec<AnsibleQuirk>)> {
    // Hand-curated list of upstream-package quirks. Empty by
    // default; populate as the adapter encounters real bugs.
    Vec::new()
}

#[derive(gen_macros::QuirkRegistry)]
#[quirks(enum_name = "AnsibleQuirk", registry_fn = "crate::quirks::registry")]
pub struct AnsibleQuirks;
