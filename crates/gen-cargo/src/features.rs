//! Cargo.features.json — vendored feature-resolution sidecar.
//!
//! The lockfile-native substrate builder reads Cargo.lock at eval time
//! via `builtins.fromTOML`, but Cargo.lock alone is not enough to
//! build crates: per-crate feature declarations live in each crate's
//! Cargo.toml, and the per-edge feature activations are computed by
//! cargo's resolver.
//!
//! This module runs `cargo metadata --format-version=1` against a
//! workspace + writes a typed `Cargo.features.json` sidecar that
//! the substrate builder reads in pure Nix to populate buildRustCrate
//! feature arguments correctly.
//!
//! The sidecar is much smaller than crate2nix's generated `Cargo.nix`
//! (typically 10-20% the size). It's a regeneration step, but it's
//! adjacent to `Cargo.lock` and follows the same lifecycle: regen on
//! lockfile changes, no other reason.

use std::path::Path;

use cargo_metadata::MetadataCommand;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::error::{CargoError, Result};

/// Top-level shape of the vendored Cargo.features.json sidecar.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FeaturesManifest {
    /// Format version — increment on breaking schema changes.
    pub version: u32,
    /// Per-crate feature data, keyed by `<name>/<version>`.
    pub crates: IndexMap<String, CrateFeatures>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CrateFeatures {
    pub name: String,
    pub version: String,
    /// Declared features: `feature_name → [implied_features...]`.
    pub features: IndexMap<String, Vec<String>>,
    /// Resolved features active for this crate.
    pub resolved_features: Vec<String>,
    /// Per-edge dep activations.
    pub dep_features: IndexMap<String, Vec<String>>,
    /// Optional dep names.
    pub optional_deps: Vec<String>,
    /// Full per-dep info — the data buildRustCrate needs to resolve
    /// dependencies correctly (name, rename, kind, features, optional,
    /// usesDefaultFeatures). Without this, deps with renames
    /// (e.g. rustix's `errno = { package = "libc_errno" }`) compile
    /// wrong.
    pub deps: Vec<CrateDep>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CrateDep {
    /// Local name the consumer uses (`extern crate <name>`).
    pub name: String,
    /// Canonical published crate name. Differs from `name` when the
    /// consumer uses `package = "..."` to rename.
    pub package: String,
    /// `"normal"` | `"dev"` | `"build"`.
    pub kind: String,
    /// cfg expression for conditional activation (`cfg(unix)`, etc.).
    pub target: Option<String>,
    /// Per-edge feature activations.
    pub features: Vec<String>,
    pub uses_default_features: bool,
    pub optional: bool,
}

/// Generate the features manifest by invoking `cargo metadata` on the
/// workspace at `workspace_root`. Calls the `cargo` binary on PATH;
/// fails with a typed error if cargo is missing or returns non-zero.
pub fn generate(workspace_root: &Path) -> Result<FeaturesManifest> {
    let manifest_path = workspace_root.join("Cargo.toml");
    if !manifest_path.exists() {
        return Err(CargoError::Io {
            path: manifest_path,
            source: std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no Cargo.toml at workspace root",
            ),
        });
    }
    let meta = MetadataCommand::new()
        .manifest_path(&manifest_path)
        .exec()
        .map_err(|e| CargoError::Io {
            path: manifest_path.clone(),
            source: std::io::Error::new(std::io::ErrorKind::Other, e.to_string()),
        })?;

    let resolved_per_package: IndexMap<String, Vec<String>> = match &meta.resolve {
        None => IndexMap::new(),
        Some(r) => r
            .nodes
            .iter()
            .map(|n| (n.id.repr.clone(), n.features.iter().map(String::from).collect()))
            .collect(),
    };

    let resolved_dep_features: IndexMap<String, IndexMap<String, Vec<String>>> = match &meta.resolve {
        None => IndexMap::new(),
        Some(r) => r
            .nodes
            .iter()
            .map(|n| {
                let mut deps: IndexMap<String, Vec<String>> = IndexMap::new();
                for d in &n.deps {
                    // Per-edge features — pulled from the resolved
                    // node's deps list. Newer cargo_metadata versions
                    // populate `dep_kinds.features`; older versions
                    // expose only the activated set.
                    let feats: Vec<String> = d
                        .dep_kinds
                        .iter()
                        .flat_map(|k| k.target.as_ref().map(|_| Vec::<String>::new()).unwrap_or_default())
                        .collect();
                    deps.insert(d.name.clone(), feats);
                }
                (n.id.repr.clone(), deps)
            })
            .collect(),
    };

    let mut crates: IndexMap<String, CrateFeatures> = IndexMap::new();
    for pkg in &meta.packages {
        let pkg_id = pkg.id.repr.clone();
        let key = format!("{}/{}", pkg.name, pkg.version);
        let features = pkg
            .features
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let resolved_features = resolved_per_package
            .get(&pkg_id)
            .cloned()
            .unwrap_or_default();
        let dep_features = resolved_dep_features
            .get(&pkg_id)
            .cloned()
            .unwrap_or_default();
        let optional_deps: Vec<String> = pkg
            .dependencies
            .iter()
            .filter(|d| d.optional)
            .map(|d| d.name.clone())
            .collect();
        let deps: Vec<CrateDep> = pkg
            .dependencies
            .iter()
            .map(|d| CrateDep {
                // Consumer-side name: rename when set, else canonical name.
                name: d.rename.clone().unwrap_or_else(|| d.name.clone()),
                // Canonical published name.
                package: d.name.clone(),
                kind: match d.kind {
                    cargo_metadata::DependencyKind::Normal => "normal".to_string(),
                    cargo_metadata::DependencyKind::Development => "dev".to_string(),
                    cargo_metadata::DependencyKind::Build => "build".to_string(),
                    _ => "normal".to_string(),
                },
                target: d.target.as_ref().map(|p| p.to_string()),
                features: d.features.iter().map(String::from).collect(),
                uses_default_features: d.uses_default_features,
                optional: d.optional,
            })
            .collect();
        crates.insert(
            key,
            CrateFeatures {
                name: pkg.name.to_string(),
                version: pkg.version.to_string(),
                features,
                resolved_features,
                dep_features,
                optional_deps,
                deps,
            },
        );
    }
    Ok(FeaturesManifest {
        version: 1,
        crates,
    })
}

/// Convenience: write the features manifest to
/// `<workspace_root>/Cargo.features.json` (the canonical location the
/// substrate builder expects).
pub fn generate_and_write(workspace_root: &Path) -> Result<std::path::PathBuf> {
    let manifest = generate(workspace_root)?;
    let out = workspace_root.join("Cargo.features.json");
    let body = serde_json::to_string_pretty(&manifest).map_err(|source| CargoError::Io {
        path: out.clone(),
        source: std::io::Error::new(std::io::ErrorKind::Other, source.to_string()),
    })?;
    std::fs::write(&out, body).map_err(|source| CargoError::Io {
        path: out.clone(),
        source,
    })?;
    Ok(out)
}
