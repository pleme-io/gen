//! Typed quirk registry for helm. Implements
//! `gen_types::QuirkRegistry` via `#[derive(QuirkRegistry)]`.
//!
//! Each registered entry names an upstream helm package that needs
//! a known-good build-time workaround. The substrate consumer's
//! `helm-quirk-apply.nix` dispatches mechanically on the variant
//! tags. Adding a new entry: append to `registry()` below.

use serde::{Deserialize, Serialize};

/// Typed quirks for known third-party upstream helm charts.
/// Each variant maps to a Nix dispatch arm in
/// `substrate/lib/build/helm/quirk-apply.nix`.
// Note: `Eq` is intentionally NOT derived — variants carry
// `serde_json::Value` which can hold floats (no Eq impl).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum HelmQuirk {
    /// Override a values.yaml entry at chart-render time. Used when
    /// an upstream chart's default value is broken under the FluxCD
    /// reconciler's posture (e.g. missing a required image tag).
    OverrideValue { path: String, value: serde_json::Value },
    /// Skip the chart's pre-install or post-install Hook resource —
    /// some charts ship hooks that require cluster privileges
    /// pleme-io's RBAC doesn't grant.
    SkipHook { hook: String },
    /// Patch a rendered manifest by JSONPath replacement. Heavier
    /// than OverrideValue (which patches at values-merge time); used
    /// when the chart's templating engine doesn't expose the field.
    PatchManifest { resource_kind: String, name: String, jsonpath: String, value: serde_json::Value },
    /// Force a specific image tag — convenient when a chart's
    /// default tag has been pulled or has known CVEs.
    ForceImageTag { container: String, tag: String },
}

pub fn registry() -> Vec<(&'static str, Vec<HelmQuirk>)> {
    // Hand-curated list of upstream-package quirks. Empty by
    // default; populate as the adapter encounters real bugs.
    Vec::new()
}

#[derive(gen_macros::QuirkRegistry)]
#[quirks(enum_name = "HelmQuirk", registry_fn = "crate::quirks::registry")]
pub struct HelmQuirks;
