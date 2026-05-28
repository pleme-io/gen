//! Typed build spec for poetry. Implements `gen_types::Spec`
//! via `#[derive(SpecShape)]`.

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

/// Pre-shaped builder args for one package. Substrate spreads this
/// verbatim into the ecosystem's nixpkgs builder. Adapter authors
/// fill in fields matching `buildXxxPackage`'s mkArgs signature.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PackageArgs {
    // TODO: add ecosystem-specific fields here.
}
