//! Typed build spec for swift — mirrors nixpkgs `swiftPackage`
//! kwargs (Swift Package Manager builds) so substrate's Swift-side
//! lockfile-builder spreads the args verbatim into the builder.
//!
//! nixpkgs reference:
//! `pkgs/development/compilers/swift/build-package.nix`.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize, gen_macros::SpecShape)]
#[spec(
    args = "PackageArgs",
    quirk = "crate::quirks::SwiftQuirk",
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
    pub quirks: Vec<crate::quirks::SwiftQuirk>,
}

/// Pre-shaped `swiftPackage` kwargs. Field names match nixpkgs'
/// builder signature so substrate spreads verbatim.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PackageArgs {
    pub pname: Option<String>,
    pub version: Option<String>,
    /// FOD hash of the resolved Package.resolved dep tree.
    #[serde(rename = "swiftDeps", skip_serializing_if = "Option::is_none")]
    pub swift_deps: Option<String>,
    /// Build configuration — `"release"` (default) or `"debug"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub configuration: Option<String>,
    /// SPM products to build (executables + libraries). Default
    /// (empty) builds everything declared in Package.swift.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub products: Vec<String>,
    /// SPM targets to limit the build to. Default = all.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub targets: Vec<String>,
    /// Minimum platform version (e.g. `"macOS 13"`).
    #[serde(rename = "swiftPlatformVersion", skip_serializing_if = "Option::is_none")]
    pub swift_platform_version: Option<String>,
    /// Linker flags forwarded to `swift build -Xlinker …`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ldflags: Vec<String>,
    /// pkg-config deps the SPM build needs.
    #[serde(rename = "pkgConfigDeps", skip_serializing_if = "Vec::is_empty")]
    pub pkg_config_deps: Vec<String>,
    /// Disable check phase. nixpkgs default = true.
    #[serde(rename = "doCheck", skip_serializing_if = "Option::is_none")]
    pub do_check: Option<bool>,
    /// Native build deps.
    #[serde(rename = "nativeBuildInputs", skip_serializing_if = "Vec::is_empty")]
    pub native_build_inputs: Vec<String>,
    /// Link-time deps.
    #[serde(rename = "buildInputs", skip_serializing_if = "Vec::is_empty")]
    pub build_inputs: Vec<String>,
}
