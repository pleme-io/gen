//! Typed build spec for poetry — mirrors poetry2nix
//! `mkPoetryApplication` / `mkPoetryEnv` kwargs so substrate's
//! Python-side lockfile-builder spreads the args verbatim.
//!
//! poetry2nix reference:
//! `https://github.com/nix-community/poetry2nix#mkPoetryApplication`.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize, gen_macros::SpecShape)]
#[spec(
    args = "PackageArgs",
    quirk = "crate::quirks::PoetryQuirk",
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
    pub quirks: Vec<crate::quirks::PoetryQuirk>,
}

/// Pre-shaped `mkPoetryApplication` / `mkPoetryEnv` kwargs. Field
/// names match poetry2nix's signature so substrate spreads verbatim.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PackageArgs {
    /// Project root containing `pyproject.toml` + `poetry.lock`.
    #[serde(rename = "projectDir", skip_serializing_if = "Option::is_none")]
    pub project_dir: Option<String>,
    /// Python interpreter attr name (e.g. `python311`). None =
    /// poetry2nix default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub python: Option<String>,
    /// Optional poetry groups to include (e.g. `["main", "production"]`).
    /// Default: all non-dev groups.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<String>,
    /// Project extras to enable (e.g. `["postgres", "redis"]`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub extras: Vec<String>,
    /// Prefer wheels over sdists when both are available. Default false.
    #[serde(rename = "preferWheels", skip_serializing_if = "Option::is_none")]
    pub prefer_wheels: Option<bool>,
    /// Editable installs (path-deps) — name -> path inside project.
    #[serde(rename = "editablePackageSources", skip_serializing_if = "IndexMap::is_empty")]
    pub editable_package_sources: IndexMap<String, String>,
    /// Disable the check phase (poetry2nix default = true).
    #[serde(rename = "doCheck", skip_serializing_if = "Option::is_none")]
    pub do_check: Option<bool>,
    /// Extra native deps not declared in pyproject.toml.
    #[serde(rename = "nativeBuildInputs", skip_serializing_if = "Vec::is_empty")]
    pub native_build_inputs: Vec<String>,
    /// Extra link-time deps.
    #[serde(rename = "buildInputs", skip_serializing_if = "Vec::is_empty")]
    pub build_inputs: Vec<String>,
}
