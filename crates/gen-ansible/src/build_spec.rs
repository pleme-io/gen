//! Typed build spec for ansible — mirrors `galaxy.yml` schema for
//! Ansible collections; substrate's `ansible-galaxy collection build`
//! wrapper consumes the rendered YAML.
//!
//! Ansible reference:
//! `https://docs.ansible.com/ansible/latest/dev_guide/collections_galaxy_meta.html`.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize, gen_macros::SpecShape)]
#[spec(
    args = "PackageArgs",
    quirk = "crate::quirks::AnsibleQuirk",
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
    pub quirks: Vec<crate::quirks::AnsibleQuirk>,
}

/// Pre-shaped `galaxy.yml` kwargs. Field names match Ansible's
/// collection-metadata schema so substrate renders verbatim into
/// the on-disk YAML the `ansible-galaxy` CLI consumes.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PackageArgs {
    pub namespace: Option<String>,
    pub name: Option<String>,
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub authors: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub license: Vec<String>,
    /// `license_file` for non-SPDX licenses — mutually exclusive
    /// with `license` per galaxy.yml schema.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license_file: Option<String>,
    /// Tag list for galaxy search (e.g. `["cloud", "aws"]`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Cross-collection dep map — `namespace.collection: version-spec`.
    #[serde(skip_serializing_if = "IndexMap::is_empty")]
    pub dependencies: IndexMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issues: Option<String>,
    /// Path patterns the build phase should NOT bundle into the
    /// collection tarball (e.g. `["tests/output", "*.swp"]`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub build_ignore: Vec<String>,
}
