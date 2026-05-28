//! Typed build spec for npm — mirrors nixpkgs `buildNpmPackage`'s
//! kwargs surface so substrate's lockfile-builder can spread the
//! emitted args verbatim into the builder.
//!
//! The `Spec` trait impl lands via `#[derive(SpecShape)]`; the
//! Quirk dispatch lands via the typed `NpmQuirk` enum.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

/// SCHEMA_VERSION = 1: initial npm BuildSpec shape. Bump on any
/// breaking field change.
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize, gen_macros::SpecShape)]
#[spec(
    args = "PackageArgs",
    quirk = "crate::quirks::NpmQuirk",
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quirks: Vec<crate::quirks::NpmQuirk>,
}

/// Pre-shaped `buildNpmPackage` kwargs. Field names match
/// nixpkgs' mkArgs signature (camelCase via serde rename) so
/// substrate's lockfile-builder spreads this verbatim.
///
/// nixpkgs reference:
/// `pkgs/build-support/node/build-npm-package/default.nix`
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PackageArgs {
    /// Name of the package (`pname` in nixpkgs convention).
    pub pname: Option<String>,
    /// Package version.
    pub version: Option<String>,
    /// FOD content hash of the prefetched `node_modules` tar (set by
    /// `prefetch-npm-deps`). Empty string when computed lazily by
    /// the substrate IFD.
    #[serde(rename = "npmDepsHash", skip_serializing_if = "Option::is_none")]
    pub npm_deps_hash: Option<String>,
    /// npm script to run for the build step (default: `build`).
    /// Empty disables the build phase entirely (publish-as-source).
    #[serde(rename = "npmBuildScript", skip_serializing_if = "Option::is_none")]
    pub npm_build_script: Option<String>,
    /// Skip-list of npm scripts that shouldn't run inside the
    /// nixpkgs sandbox (e.g. `prepublish`, `postinstall`-side-effect
    /// scripts).
    #[serde(rename = "dontNpmBuild", skip_serializing_if = "Option::is_none")]
    pub dont_npm_build: Option<bool>,
    /// Don't run `npm prune` after install (some monorepos need
    /// every dep for the build, including dev).
    #[serde(rename = "dontNpmPrune", skip_serializing_if = "Option::is_none")]
    pub dont_npm_prune: Option<bool>,
    /// Pass `--legacy-peer-deps` to npm. Required for many React
    /// ecosystem packages with strict peerDep ranges.
    #[serde(rename = "npmFlags", skip_serializing_if = "Vec::is_empty")]
    pub npm_flags: Vec<String>,
    /// Override the nodejs binary used. Default: `nodejs_LTS`.
    /// String form so substrate can resolve to a pkgs attr (e.g.
    /// "nodejs_20"); None = nixpkgs default.
    #[serde(rename = "nodejs", skip_serializing_if = "Option::is_none")]
    pub nodejs: Option<String>,
    /// Native build deps (gcc, python for node-gyp, etc).
    #[serde(rename = "nativeBuildInputs", skip_serializing_if = "Vec::is_empty")]
    pub native_build_inputs: Vec<String>,
    /// Runtime / link-time deps.
    #[serde(rename = "buildInputs", skip_serializing_if = "Vec::is_empty")]
    pub build_inputs: Vec<String>,
    /// Allow npm cache writes during build (some workflows need it).
    #[serde(rename = "makeCacheWritable", skip_serializing_if = "Option::is_none")]
    pub make_cache_writable: Option<bool>,
    /// Force the prefetched cache to be empty (development /
    /// debug). Substrate normally leaves this unset.
    #[serde(rename = "forceEmptyCache", skip_serializing_if = "Option::is_none")]
    pub force_empty_cache: Option<bool>,
}
