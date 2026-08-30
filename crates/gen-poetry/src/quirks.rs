//! Typed quirk registry for poetry. Implements
//! `gen_types::QuirkRegistry` via `#[derive(QuirkRegistry)]`.
//!
//! Each registered entry names an upstream poetry package that needs
//! a known-good build-time workaround. The substrate consumer's
//! `poetry-quirk-apply.nix` dispatches mechanically on the variant
//! tags. Adding a new entry: append to `registry()` below.

use serde::{Deserialize, Serialize};

/// Typed quirks for known third-party upstream poetry packages.
/// Each variant maps to a Nix dispatch arm in
/// `substrate/lib/build/poetry/quirk-apply.nix`.
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
pub enum PoetryQuirk {
    /// Override the build-system backend for a wheel that ships
    /// broken `[build-system]` metadata.
    OverrideBuildSystem { package: String, backend: String },
    /// Inject a poetry2nix `defaultPoetryOverrides` arm — apply a
    /// `prev.<package>.overridePythonAttrs` patch declaratively.
    OverrideAttrs {
        package: String,
        attr: String,
        value: String,
    },
    /// Skip the package's check phase.
    SkipCheck { package: String },
    /// Force wheel-or-sdist preference per-package — overrides the
    /// global `preferWheels` setting.
    PreferWheel { package: String, prefer: bool },
}

pub fn registry() -> Vec<(&'static str, Vec<PoetryQuirk>)> {
    // Hand-curated list of upstream-package quirks. Empty by
    // default; populate as the adapter encounters real bugs.
    Vec::new()
}

#[derive(gen_macros::QuirkRegistry)]
#[quirks(enum_name = "PoetryQuirk", registry_fn = "crate::quirks::registry")]
pub struct PoetryQuirks;

// Fleet-wide dispatcher-catalog registration.
gen_platform::register_dispatcher!("gen.poetry.poetry-quirk", PoetryQuirk);
