//! Typed build spec for pip — mirrors nixpkgs `buildPythonPackage`
//! kwargs so substrate's Python-side lockfile-builder spreads the
//! args verbatim into the builder.
//!
//! nixpkgs reference: `pkgs/development/interpreters/python/mk-python-derivation.nix`.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize, gen_macros::SpecShape)]
#[spec(
    args = "PackageArgs",
    quirk = "crate::quirks::PipQuirk",
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
    pub quirks: Vec<crate::quirks::PipQuirk>,
}

/// Pre-shaped `buildPythonPackage` kwargs. Field names match nixpkgs'
/// `mkPythonDerivation` signature (camelCase via serde rename) so
/// substrate spreads verbatim into the builder.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PackageArgs {
    pub pname: Option<String>,
    pub version: Option<String>,
    /// PEP 517 / pyproject.toml builds (true) vs legacy setup.py (false).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pyproject: Option<bool>,
    /// PEP 517 build backends (e.g. `["setuptools"]`, `["flit-core"]`,
    /// `["poetry-core"]`, `["hatchling"]`). Required when `pyproject = true`.
    #[serde(rename = "build-system", skip_serializing_if = "Vec::is_empty")]
    pub build_system: Vec<String>,
    /// Runtime dependencies — propagated to consumers of this package.
    #[serde(
        rename = "propagatedBuildInputs",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub propagated_build_inputs: Vec<String>,
    /// Build-time native deps (compilers, codegen tools).
    #[serde(rename = "nativeBuildInputs", skip_serializing_if = "Vec::is_empty")]
    pub native_build_inputs: Vec<String>,
    /// Build-time link/include deps (libxml2, openssl, etc).
    #[serde(rename = "buildInputs", skip_serializing_if = "Vec::is_empty")]
    pub build_inputs: Vec<String>,
    /// Disable check phase. Default (None) = nixpkgs default = true.
    #[serde(rename = "doCheck", skip_serializing_if = "Option::is_none")]
    pub do_check: Option<bool>,
    /// Smoke-test imports — `pythonImportsCheck = ["pkg.mod"]` runs
    /// `python -c 'import pkg.mod'` post-install.
    #[serde(rename = "pythonImportsCheck", skip_serializing_if = "Vec::is_empty")]
    pub python_imports_check: Vec<String>,
    /// Check-only deps (pytest, hypothesis, etc).
    #[serde(rename = "nativeCheckInputs", skip_serializing_if = "Vec::is_empty")]
    pub native_check_inputs: Vec<String>,
    /// Legacy `format = "wheel" | "setuptools" | "pyproject"`. New
    /// code should set `pyproject = true` instead; this is kept for
    /// pre-pyproject intake.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
}
