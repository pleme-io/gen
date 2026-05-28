//! Typed build spec for gomod — mirrors nixpkgs `buildGoModule`
//! kwargs so substrate's Go-side lockfile-builder spreads the args
//! verbatim into the builder.
//!
//! nixpkgs reference:
//! `pkgs/build-support/go/module.nix` (`buildGoModule`).

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize, gen_macros::SpecShape)]
#[spec(
    args = "PackageArgs",
    quirk = "crate::quirks::GomodQuirk",
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
    pub quirks: Vec<crate::quirks::GomodQuirk>,
}

/// Pre-shaped `buildGoModule` kwargs. Field names match nixpkgs'
/// builder signature (camelCase via serde rename) so substrate
/// spreads verbatim.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PackageArgs {
    pub pname: Option<String>,
    pub version: Option<String>,
    /// FOD hash of the vendored dep tarball. `null` (Nix) =
    /// `vendor/` directory already in source; `Some("sha256-…")` =
    /// proxy-fetched.
    #[serde(rename = "vendorHash", skip_serializing_if = "Option::is_none")]
    pub vendor_hash: Option<String>,
    /// Run `go mod vendor` against the proxy (true) vs git tarballs
    /// (false, default).
    #[serde(rename = "proxyVendor", skip_serializing_if = "Option::is_none")]
    pub proxy_vendor: Option<bool>,
    /// Build tags forwarded to `go build -tags …`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// `go build -ldflags …` entries.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ldflags: Vec<String>,
    /// Subpackages to build — paths relative to the module root.
    /// Default (empty) builds everything under `./...`.
    #[serde(rename = "subPackages", skip_serializing_if = "Vec::is_empty")]
    pub sub_packages: Vec<String>,
    /// Disable check phase. nixpkgs default = true.
    #[serde(rename = "doCheck", skip_serializing_if = "Option::is_none")]
    pub do_check: Option<bool>,
    /// Environment variables — e.g. `CGO_ENABLED=0`, `GOOS=linux`.
    #[serde(skip_serializing_if = "IndexMap::is_empty")]
    pub env: IndexMap<String, String>,
    /// Build-time native deps (cgo headers, generators).
    #[serde(rename = "nativeBuildInputs", skip_serializing_if = "Vec::is_empty")]
    pub native_build_inputs: Vec<String>,
    /// Link-time deps.
    #[serde(rename = "buildInputs", skip_serializing_if = "Vec::is_empty")]
    pub build_inputs: Vec<String>,
}
