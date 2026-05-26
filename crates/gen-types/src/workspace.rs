//! Typed [`Workspace`] — multi-package project root.
//!
//! Multi-package projects (Cargo `[workspace]`, npm workspaces, pnpm
//! workspace, Composer monorepo, Bundler `Gemfile`-with-multiple-
//! gemspecs, …) all reduce to: a root + N member-package paths +
//! shared workspace-level metadata.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Typed workspace root + the IDs of every member package contained
/// in it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workspace {
    pub root: PathBuf,
    /// Member packages — relative paths from `root`.
    pub members: Vec<PathBuf>,
    /// Adapter that produced this workspace (`"cargo"`, `"npm"`, …).
    pub adapter: String,
    /// Shared workspace-level metadata that members inherit when
    /// they don't override it (Cargo `workspace.package`,
    /// Composer monorepo, …).
    #[serde(default)]
    pub shared_metadata: indexmap::IndexMap<String, String>,
}

impl Workspace {
    /// Convenience constructor for the common case (no shared
    /// metadata).
    #[must_use]
    pub fn new(
        root: impl Into<PathBuf>,
        members: Vec<PathBuf>,
        adapter: impl Into<String>,
    ) -> Self {
        Self {
            root: root.into(),
            members,
            adapter: adapter.into(),
            shared_metadata: indexmap::IndexMap::new(),
        }
    }

    /// Single-package workspace shape (e.g. a single-crate cargo
    /// repo). Members is `[root]`.
    #[must_use]
    pub fn single_package(root: impl Into<PathBuf>, adapter: impl Into<String>) -> Self {
        let root = root.into();
        Self::new(root.clone(), vec![root], adapter)
    }

    /// True if this workspace has multiple member packages.
    #[must_use]
    pub fn is_multi_package(&self) -> bool {
        self.members.len() > 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_package_workspace_isnt_multi() {
        let w = Workspace::single_package("/x", "cargo");
        assert!(!w.is_multi_package());
    }

    #[test]
    fn multi_member_workspace_is_multi() {
        let w = Workspace::new(
            "/x",
            vec![PathBuf::from("a"), PathBuf::from("b")],
            "cargo",
        );
        assert!(w.is_multi_package());
    }

    #[test]
    fn round_trip_through_serde() {
        let w = Workspace::new(
            "/x",
            vec![PathBuf::from("a"), PathBuf::from("b")],
            "cargo",
        );
        let j = serde_json::to_string(&w).unwrap();
        let parsed: Workspace = serde_json::from_str(&j).unwrap();
        assert_eq!(w, parsed);
    }
}
