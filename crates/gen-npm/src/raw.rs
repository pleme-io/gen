//! Raw serde shapes mirroring the npm + pnpm on-disk formats.

use indexmap::IndexMap;
use serde::Deserialize;

/// `package.json`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct PackageJson {
    pub name: Option<String>,
    pub version: Option<String>,
    pub description: Option<String>,
    pub license: Option<String>,
    pub homepage: Option<String>,
    #[serde(default)]
    pub dependencies: IndexMap<String, String>,
    #[serde(default, rename = "devDependencies")]
    pub dev_dependencies: IndexMap<String, String>,
    #[serde(default, rename = "peerDependencies")]
    pub peer_dependencies: IndexMap<String, String>,
    #[serde(default, rename = "optionalDependencies")]
    pub optional_dependencies: IndexMap<String, String>,
    #[serde(default)]
    pub author: Option<AuthorField>,
    #[serde(default)]
    pub contributors: Vec<AuthorField>,
    #[serde(default)]
    pub repository: Option<RepositoryField>,
    /// Either an array of paths or `{ packages: [...] }`.
    #[serde(default)]
    pub workspaces: Option<WorkspacesField>,
}

impl PackageJson {
    pub fn workspaces_paths(&self) -> Vec<String> {
        match &self.workspaces {
            None => Vec::new(),
            Some(WorkspacesField::Array(v)) => v.clone(),
            Some(WorkspacesField::Object { packages }) => packages.clone(),
        }
    }

    pub fn author_strings(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .contributors
            .iter()
            .map(AuthorField::to_string_form)
            .collect();
        if let Some(a) = &self.author {
            out.insert(0, a.to_string_form());
        }
        out
    }

    pub fn repository_url(&self) -> Option<String> {
        match &self.repository {
            None => None,
            Some(RepositoryField::Url(s)) => Some(s.clone()),
            Some(RepositoryField::Object { url, .. }) => url.clone(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum AuthorField {
    String(String),
    Object {
        name: Option<String>,
        email: Option<String>,
        url: Option<String>,
    },
}

impl AuthorField {
    pub fn to_string_form(&self) -> String {
        match self {
            Self::String(s) => s.clone(),
            Self::Object { name, email, url } => {
                let mut s = name.clone().unwrap_or_default();
                if let Some(e) = email {
                    s.push_str(&format!(" <{e}>"));
                }
                if let Some(u) = url {
                    s.push_str(&format!(" ({u})"));
                }
                s
            }
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum RepositoryField {
    Url(String),
    Object {
        url: Option<String>,
        #[serde(rename = "type")]
        _type: Option<String>,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum WorkspacesField {
    Array(Vec<String>),
    Object { packages: Vec<String> },
}

// ── package-lock.json ────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct PackageLock {
    #[serde(default)]
    pub packages: IndexMap<String, PackageLockEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PackageLockEntry {
    pub name: Option<String>,
    pub version: Option<String>,
    pub resolved: Option<String>,
    pub integrity: Option<String>,
}

// ── pnpm-lock.yaml ───────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct PnpmLock {
    #[serde(default)]
    pub packages: IndexMap<String, PnpmPackageEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PnpmPackageEntry {
    pub resolution: Option<PnpmResolution>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PnpmResolution {
    pub integrity: Option<String>,
    #[serde(default)]
    pub tarball: Option<String>,
}
