//! Typed [`Manifest`] — top-level adapter output.
//!
//! Every adapter (`gen-cargo`, `gen-npm`, `gen-bundler`, …) emits
//! one of these per workspace; every renderer consumes them.

use crate::{Lockfile, Package, Workspace};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// One parsed workspace + every package in it + the lockfile (if
/// present).
///
/// This is the "common currency" the gen ecosystem trades in.
/// Engine APIs all flow through `Manifest` values.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub workspace_root: PathBuf,
    pub workspace: Workspace,
    pub packages: Vec<Package>,
    #[serde(default)]
    pub lockfile: Option<Lockfile>,
}

impl Manifest {
    /// Construct a fresh manifest from its parts. Adapters call this
    /// at the end of their parse fn.
    #[must_use]
    pub fn new(
        workspace_root: impl Into<PathBuf>,
        workspace: Workspace,
        packages: Vec<Package>,
        lockfile: Option<Lockfile>,
    ) -> Self {
        Self {
            workspace_root: workspace_root.into(),
            workspace,
            packages,
            lockfile,
        }
    }

    /// Count packages in this manifest.
    #[must_use]
    pub fn package_count(&self) -> usize {
        self.packages.len()
    }

    /// Find a package by name (first match).
    #[must_use]
    pub fn find_package(&self, name: &str) -> Option<&Package> {
        self.packages.iter().find(|p| p.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PackageSource, Registry, Version};

    #[test]
    fn manifest_construction_and_lookup() {
        let p = Package {
            name: "demo".into(),
            version: Version::new(0, 1, 0),
            source: PackageSource::Path { path: ".".into() },
            registry: Registry::None,
            dependencies: vec![],
            features: vec![],
            build_steps: vec![],
            license: None,
            description: None,
            authors: vec![],
            homepage: None,
            repository: None,
        };
        let m = Manifest::new(
            "/x",
            Workspace::single_package("/x", "cargo"),
            vec![p],
            None,
        );
        assert_eq!(m.package_count(), 1);
        assert!(m.find_package("demo").is_some());
        assert!(m.find_package("missing").is_none());
    }

    #[test]
    fn round_trip_through_serde() {
        let m = Manifest::new(
            "/x",
            Workspace::single_package("/x", "cargo"),
            vec![],
            None,
        );
        let j = serde_json::to_string(&m).unwrap();
        let parsed: Manifest = serde_json::from_str(&j).unwrap();
        assert_eq!(m, parsed);
    }
}
