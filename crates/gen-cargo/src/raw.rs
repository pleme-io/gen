//! Raw serde shapes that mirror Cargo's on-disk TOML. These exist
//! only as the deserialization surface — they are converted to the
//! typed [`gen_types`] IR by [`crate::convert`]. Keep this module
//! straight serde with no business logic; bugs in the conversion
//! belong in `convert.rs`, not here.

use indexmap::IndexMap;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct CargoToml {
    #[serde(default)]
    pub package: Option<RawPackage>,
    #[serde(default)]
    pub workspace: Option<RawWorkspace>,
    #[serde(default)]
    pub dependencies: IndexMap<String, RawDep>,
    #[serde(default, rename = "dev-dependencies")]
    pub dev_dependencies: IndexMap<String, RawDep>,
    #[serde(default, rename = "build-dependencies")]
    pub build_dependencies: IndexMap<String, RawDep>,
    #[serde(default)]
    pub features: IndexMap<String, Vec<String>>,
    #[serde(default)]
    pub target: IndexMap<String, RawTargetBlock>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawPackage {
    pub name: Option<RawInheritedString>,
    pub version: Option<RawInheritedString>,
    #[serde(default)]
    pub description: Option<RawInheritedString>,
    #[serde(default)]
    pub license: Option<RawInheritedString>,
    #[serde(default)]
    pub repository: Option<RawInheritedString>,
    #[serde(default)]
    pub homepage: Option<RawInheritedString>,
    #[serde(default)]
    pub authors: Option<RawInheritedStringList>,
    #[serde(default)]
    pub edition: Option<RawInheritedString>,
    #[serde(default, rename = "rust-version")]
    pub rust_version: Option<RawInheritedString>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawWorkspace {
    #[serde(default)]
    pub members: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
    #[serde(default)]
    pub package: Option<RawPackage>,
    #[serde(default)]
    pub dependencies: IndexMap<String, RawDep>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawTargetBlock {
    #[serde(default)]
    pub dependencies: IndexMap<String, RawDep>,
    #[serde(default, rename = "dev-dependencies")]
    pub dev_dependencies: IndexMap<String, RawDep>,
    #[serde(default, rename = "build-dependencies")]
    pub build_dependencies: IndexMap<String, RawDep>,
}

/// Cargo's `foo = "1"` short form OR `foo = { version = "1", ... }`
/// long form OR `foo = { workspace = true }` inheritance form.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum RawDep {
    Short(String),
    Long(RawDepDetail),
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct RawDepDetail {
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub git: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub rev: Option<String>,
    #[serde(default)]
    pub registry: Option<String>,
    #[serde(default)]
    pub package: Option<String>,
    #[serde(default)]
    pub features: Vec<String>,
    #[serde(default = "default_true", rename = "default-features")]
    pub default_features: bool,
    #[serde(default)]
    pub optional: bool,
    #[serde(default)]
    pub workspace: bool,
}

fn default_true() -> bool {
    true
}

/// Field that may either be a literal value or `{ workspace = true }`.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum RawInheritedString {
    Literal(String),
    Inherit { workspace: bool },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum RawInheritedStringList {
    Literal(Vec<String>),
    Inherit { workspace: bool },
}

impl RawInheritedString {
    /// Resolve against a workspace-level value. Returns `None` if the
    /// field inherits but the workspace doesn't define it.
    pub fn resolve<'a>(&'a self, workspace_value: Option<&'a str>) -> Option<&'a str> {
        match self {
            Self::Literal(s) => Some(s.as_str()),
            Self::Inherit { workspace: true } => workspace_value,
            Self::Inherit { workspace: false } => None,
        }
    }
}

impl RawInheritedStringList {
    pub fn resolve<'a>(&'a self, workspace_value: Option<&'a [String]>) -> Option<&'a [String]> {
        match self {
            Self::Literal(s) => Some(s.as_slice()),
            Self::Inherit { workspace: true } => workspace_value,
            Self::Inherit { workspace: false } => None,
        }
    }
}

/// Cargo.lock — `version` + `[[package]]` entries.
///
/// Two on-disk formats:
///   - **v2+/v3** (modern): checksums live inline on each `[[package]]`
///     entry as `checksum = "..."`. Default since Cargo 2019.
///   - **v1** (legacy): checksums live in a separate `[metadata]`
///     table at the bottom keyed by
///     `"checksum <name> <version> (<source>)" = "..."`.
///
/// The deserialization captures both; [`super::convert::convert_lockfile`]
/// merges them so consumers see one consistent shape regardless of
/// which format the source repo uses.
#[derive(Debug, Clone, Deserialize)]
pub struct CargoLock {
    #[serde(default)]
    pub version: Option<u32>,
    #[serde(default, rename = "package")]
    pub packages: Vec<RawLockPackage>,
    /// Legacy v1 metadata table — keyed by
    /// `"checksum <name> <version> (<source>)"`. Empty for v2+ lockfiles.
    #[serde(default)]
    pub metadata: IndexMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawLockPackage {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub checksum: Option<String>,
    #[serde(default)]
    pub dependencies: Vec<String>,
}
