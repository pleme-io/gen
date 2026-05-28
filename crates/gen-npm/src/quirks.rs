//! Typed NpmQuirk registry — known upstream-package build-time
//! workarounds for the nixpkgs `buildNpmPackage` sandbox.
//!
//! Each variant maps to a Nix dispatch arm in
//! `substrate/lib/build/npm/quirk-apply.nix` (TBD when substrate
//! grows the npm builder).

use serde::{Deserialize, Serialize};

/// Typed quirks for known third-party upstream npm packages whose
/// install/build phase fails inside the nixpkgs sandbox without a
/// known-good workaround.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum NpmQuirk {
    /// Append a CLI flag to the `npm install` invocation. Most common
    /// use: `--legacy-peer-deps` for React ecosystem packages whose
    /// peer-dependency ranges are too strict for the resolver.
    NpmInstallFlag { flag: String },
    /// Skip a specific package's `postinstall` script — required for
    /// packages whose postinstall does network access (puppeteer,
    /// playwright-core, sharp's binary download, etc) or non-
    /// reproducible side effects inside the sandbox.
    SkipPostinstall,
    /// Force a specific Node.js major version for the build. Some
    /// older packages break on newer node; some new packages require
    /// node 18+.
    PinNodejs { version: String },
    /// Override the npm registry URL during install. Used when a
    /// package's lockfile pins a private registry that the sandbox
    /// can't reach.
    OverrideRegistry { url: String },
    /// Apply a one-line source substitution before build. npm
    /// analog of CrateQuirk::SubstituteSource — used for upstream
    /// JS bugs whose fix is trivially small.
    SubstituteSource {
        file: String,
        from: String,
        to: String,
    },
}

/// The canonical npm quirks registry. Starts empty — entries land
/// as we encounter real upstream packages that need each class.
/// Each entry should name the upstream bug + tracking issue.
pub fn registry() -> Vec<(&'static str, Vec<NpmQuirk>)> {
    Vec::new()
}

#[derive(gen_macros::QuirkRegistry)]
#[quirks(enum_name = "NpmQuirk", registry_fn = "crate::quirks::registry")]
pub struct NpmQuirks;
