//! `Cargo.build-spec.json` — the canonical complete typed build manifest.
//!
//! Composes Cargo.toml + Cargo.lock + cargo metadata into one typed
//! JSON that the substrate Nix builder consumes directly with
//! `builtins.fromJSON`. Nix becomes a pure orchestrator — no parsing,
//! no string splitting, no semantic resolution.
//!
//! This is the architectural split: Rust owns ALL semantics
//! (parsing, dep resolution, feature activation, src URL synthesis,
//! sha256 plumbing, rename handling, optional gating, target
//! conditions); Nix owns dispatch (one JSON read, per-crate
//! buildRustCrate calls, attrset assembly).

use std::path::Path;

use cargo_metadata::MetadataCommand;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::error::{CargoError, Result};

/// Schema version — bump on breaking changes.
const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BuildSpec {
    pub version: u32,
    pub workspace: WorkspaceSpec,
    pub crates: IndexMap<String, CrateSpec>,
    pub root_crate: Option<String>,
    pub workspace_members: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkspaceSpec {
    pub root: String,
    pub members: Vec<WorkspaceMemberSpec>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkspaceMemberSpec {
    pub name: String,
    /// Path relative to the workspace root.
    pub relative_path: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CrateSpec {
    pub name: String,
    pub version: String,
    pub edition: String,
    pub source: CrateSource,
    pub features: Vec<String>,
    pub proc_macro: bool,
    pub dependencies: Vec<CrateDepSpec>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum CrateSource {
    /// Registry tarball — fetchurl resolved at eval.
    Registry {
        url: String,
        sha256: String,
        /// File extension hint for nix's unpacker (`.tar.gz`).
        name_with_ext: String,
    },
    /// Git source — fetchgit resolved at eval.
    Git {
        url: String,
        rev: String,
        sha256: Option<String>,
    },
    /// Path source (workspace member) — relative to workspace root.
    Path { relative_path: String },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CrateDepSpec {
    /// Consumer-side identifier (`extern crate <name>`); equal to
    /// `package_key` after `/version` stripping when no rename.
    pub name: String,
    /// Key into BuildSpec.crates — uniquely identifies the resolved
    /// dep. Format: `<canonical-name>-<version>`.
    pub package_key: String,
    pub kind: DepKind,
    pub features: Vec<String>,
    pub uses_default_features: bool,
    pub optional: bool,
    pub target: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DepKind {
    Normal,
    Build,
    Dev,
}

/// Host target triple — the platform the spec targets unless the
/// caller overrides via `generate_for_target`.
fn host_target_triple() -> &'static str {
    // Conservative: known fleet hosts. Full triple detection lives
    // in shikumi config (M+1 enrichment).
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "aarch64-apple-darwin"
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        "x86_64-apple-darwin"
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        "x86_64-unknown-linux-gnu"
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        "aarch64-unknown-linux-gnu"
    }
    #[cfg(not(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64"),
    )))]
    {
        ""
    }
}

/// Generate the complete typed BuildSpec for the workspace at `root`,
/// targeting the host. cargo metadata is invoked with
/// `--filter-platform=<host>` so the resolve graph contains only deps
/// active for this target — the substrate Nix side never has to
/// evaluate cfg() expressions itself.
pub fn generate(root: &Path) -> Result<BuildSpec> {
    generate_for_target(root, host_target_triple())
}

/// Generate the BuildSpec for an explicit target triple. Used by
/// cross-build CI to produce a per-target spec.
pub fn generate_for_target(root: &Path, target: &str) -> Result<BuildSpec> {
    // Canonicalize the root path so the relative-path math against
    // cargo metadata's absolute paths produces the right answer.
    let root = std::fs::canonicalize(root).map_err(|source| CargoError::Io {
        path: root.to_path_buf(),
        source,
    })?;
    let root = root.as_path();
    let manifest_path = root.join("Cargo.toml");
    if !manifest_path.exists() {
        return Err(CargoError::Io {
            path: manifest_path,
            source: std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no Cargo.toml at workspace root",
            ),
        });
    }
    // Filter the resolve graph to deps active for this target.
    // cargo's resolver does the heavy lifting; we just pass --filter-platform.
    let mut cmd = MetadataCommand::new();
    cmd.manifest_path(&manifest_path);
    if !target.is_empty() {
        cmd.other_options(vec!["--filter-platform".to_string(), target.to_string()]);
    }
    let meta = cmd.exec().map_err(|e| CargoError::Io {
        path: manifest_path.clone(),
        source: std::io::Error::new(std::io::ErrorKind::Other, e.to_string()),
    })?;

    // Lockfile checksums — cargo-metadata doesn't surface them, so we
    // read Cargo.lock directly + index by (name, version). Surface any
    // parse error rather than silently producing an unbuildable spec.
    let checksums: IndexMap<(String, String), String> = {
        let manifest = crate::parse(root).map_err(|e| {
            eprintln!("gen lock-build: gen_cargo::parse failed: {e}");
            e
        })?;
        manifest
            .lockfile
            .map(|lf| {
                lf.resolved
                    .values()
                    .filter_map(|r| {
                        let h = r.integrity.as_ref()?;
                        let hex = h.strip_prefix("sha256:").unwrap_or(h);
                        Some(((r.id.name.clone(), r.id.version.to_string()), hex.to_string()))
                    })
                    .collect()
            })
            .unwrap_or_default()
    };

    let workspace_root_str = root.display().to_string();

    // Identify workspace members.
    let workspace_member_ids: Vec<_> = meta.workspace_members.iter().collect();
    let workspace_members: Vec<WorkspaceMemberSpec> = workspace_member_ids
        .iter()
        .filter_map(|id| meta.packages.iter().find(|p| &p.id == *id))
        .map(|p| {
            let abs_dir = p.manifest_path.parent().map(|p| p.to_string()).unwrap_or_default();
            let rel = pathdiff_relative(&abs_dir, &workspace_root_str)
                .unwrap_or_else(|| abs_dir.clone());
            WorkspaceMemberSpec {
                name: p.name.to_string(),
                relative_path: if rel.is_empty() { ".".to_string() } else { rel },
            }
        })
        .collect();

    let workspace_member_names: std::collections::HashSet<String> = workspace_members
        .iter()
        .map(|m| m.name.clone())
        .collect();

    // Pre-index resolved features by package id.
    let resolved_features: IndexMap<String, Vec<String>> = meta
        .resolve
        .as_ref()
        .map(|r| {
            r.nodes
                .iter()
                .map(|n| (n.id.repr.clone(), n.features.iter().map(String::from).collect()))
                .collect()
        })
        .unwrap_or_default();

    // Map of all resolved package ids → cargo_metadata Package.
    // Used to look up canonical name+version for each dep edge.
    let by_id: IndexMap<String, &cargo_metadata::Package> = meta
        .packages
        .iter()
        .map(|p| (p.id.repr.clone(), p))
        .collect();

    // Build the per-package dep edges from the resolve graph (which
    // already knows the actual resolved dep ids — no manual matching).
    let dep_edges: IndexMap<String, Vec<(String, String)>> = meta
        .resolve
        .as_ref()
        .map(|r| {
            r.nodes
                .iter()
                .map(|n| {
                    let edges: Vec<(String, String)> = n
                        .deps
                        .iter()
                        .map(|d| (d.name.clone(), d.pkg.repr.clone()))
                        .collect();
                    (n.id.repr.clone(), edges)
                })
                .collect()
        })
        .unwrap_or_default();

    let mut crates: IndexMap<String, CrateSpec> = IndexMap::new();
    for pkg in &meta.packages {
        let key = format!("{}-{}", pkg.name, pkg.version);
        let is_member = workspace_member_names.contains(pkg.name.as_str());

        // Edition: cargo-metadata provides it directly.
        let edition = format!("{}", pkg.edition);

        // proc-macro detection from cargo-metadata's targets.
        // TargetKind is a stringly-typed list in cargo_metadata 0.18.
        let proc_macro = pkg
            .targets
            .iter()
            .any(|t| t.kind.iter().any(|k| k == "proc-macro"));

        // Source resolution.
        let source = if is_member {
            let abs_dir = pkg.manifest_path.parent().map(|p| p.to_string()).unwrap_or_default();
            let rel = pathdiff_relative(&abs_dir, &workspace_root_str)
                .unwrap_or_else(|| abs_dir.clone());
            CrateSource::Path {
                relative_path: if rel.is_empty() { ".".to_string() } else { rel },
            }
        } else if let Some(src) = &pkg.source {
            let src_str = src.to_string();
            if src_str.starts_with("registry+") {
                let sha = checksums
                    .get(&(pkg.name.to_string(), pkg.version.to_string()))
                    .cloned()
                    .unwrap_or_default();
                if sha.is_empty() {
                    eprintln!(
                        "gen lock-build: missing checksum for {}/{}",
                        pkg.name, pkg.version
                    );
                }
                CrateSource::Registry {
                    url: format!(
                        "https://crates.io/api/v1/crates/{}/{}/download",
                        pkg.name, pkg.version
                    ),
                    sha256: sha,
                    name_with_ext: format!("{}-{}.tar.gz", pkg.name, pkg.version),
                }
            } else if src_str.starts_with("git+") {
                let trimmed = src_str.trim_start_matches("git+");
                let (url, rev) = trimmed
                    .rsplit_once('#')
                    .map(|(u, f)| (u.to_string(), f.to_string()))
                    .unwrap_or_else(|| (trimmed.to_string(), String::new()));
                CrateSource::Git {
                    url,
                    rev,
                    sha256: None,
                }
            } else {
                // Path source not in workspace — bare relative.
                let abs_dir = pkg.manifest_path.parent().map(|p| p.to_string()).unwrap_or_default();
                let rel = pathdiff_relative(&abs_dir, &workspace_root_str)
                    .unwrap_or_else(|| abs_dir.clone());
                CrateSource::Path { relative_path: rel }
            }
        } else {
            CrateSource::Path {
                relative_path: ".".to_string(),
            }
        };

        let features = resolved_features
            .get(&pkg.id.repr)
            .cloned()
            .unwrap_or_default();

        // Build typed dep edges using the resolve graph (authoritative
        // for "what is in the closure") combined with each dep's
        // declared kind/features/optional from the Cargo.toml side.
        let edges_for_pkg: Vec<(String, String)> = dep_edges
            .get(&pkg.id.repr)
            .cloned()
            .unwrap_or_default();

        let mut dependencies: Vec<CrateDepSpec> = Vec::new();
        for (local_name, dep_pkg_id) in &edges_for_pkg {
            let Some(dep_pkg) = by_id.get(dep_pkg_id) else { continue; };
            let package_key = format!("{}-{}", dep_pkg.name, dep_pkg.version);

            // Look up the consumer's declared dep entry to recover
            // features + optional + kind + target. cargo metadata
            // shows multiple entries for the same dep name when it
            // appears in normal+build+dev — we pick the first non-dev
            // edge that matches the canonical name.
            let declared = pkg
                .dependencies
                .iter()
                .find(|d| {
                    let consumer_name = d.rename.clone().unwrap_or_else(|| d.name.clone());
                    &consumer_name == local_name
                        && d.kind != cargo_metadata::DependencyKind::Development
                });

            let (kind, features, uses_default_features, optional, target) =
                if let Some(d) = declared {
                    let k = match d.kind {
                        cargo_metadata::DependencyKind::Normal => DepKind::Normal,
                        cargo_metadata::DependencyKind::Build => DepKind::Build,
                        cargo_metadata::DependencyKind::Development => DepKind::Dev,
                        _ => DepKind::Normal,
                    };
                    (
                        k,
                        d.features.iter().map(String::from).collect(),
                        d.uses_default_features,
                        d.optional,
                        d.target.as_ref().map(|p| p.to_string()),
                    )
                } else {
                    (DepKind::Normal, Vec::new(), true, false, None)
                };

            // Dev deps are not part of the runtime closure; skip.
            if matches!(kind, DepKind::Dev) {
                continue;
            }

            dependencies.push(CrateDepSpec {
                name: local_name.clone(),
                package_key,
                kind,
                features,
                uses_default_features,
                optional,
                target,
            });
        }

        crates.insert(
            key,
            CrateSpec {
                name: pkg.name.to_string(),
                version: pkg.version.to_string(),
                edition,
                source,
                features,
                proc_macro,
                dependencies,
            },
        );
    }

    let root_crate = meta
        .root_package()
        .map(|p| format!("{}-{}", p.name, p.version))
        .or_else(|| workspace_members.first().map(|m| {
            let pkg = meta.packages.iter().find(|p| p.name.as_str() == m.name);
            match pkg {
                Some(p) => format!("{}-{}", p.name, p.version),
                None => String::new(),
            }
        }));

    let workspace_member_keys: Vec<String> = workspace_members
        .iter()
        .filter_map(|m| {
            let pkg = meta.packages.iter().find(|p| p.name.as_str() == m.name)?;
            Some(format!("{}-{}", pkg.name, pkg.version))
        })
        .collect();

    Ok(BuildSpec {
        version: SCHEMA_VERSION,
        workspace: WorkspaceSpec {
            root: workspace_root_str,
            members: workspace_members,
        },
        crates,
        root_crate,
        workspace_members: workspace_member_keys,
    })
}

pub fn generate_and_write(root: &Path) -> Result<std::path::PathBuf> {
    let spec = generate(root)?;
    let out = root.join("Cargo.build-spec.json");
    let body = serde_json::to_string_pretty(&spec).map_err(|e| CargoError::Io {
        path: out.clone(),
        source: std::io::Error::new(std::io::ErrorKind::Other, e.to_string()),
    })?;
    std::fs::write(&out, body).map_err(|source| CargoError::Io {
        path: out.clone(),
        source,
    })?;
    Ok(out)
}

/// Compute relative path from `from` to `base`. Returns None if
/// `from` doesn't start with `base`. Both inputs are display paths.
fn pathdiff_relative(from: &str, base: &str) -> Option<String> {
    let base_trim = base.trim_end_matches('/');
    if from == base_trim {
        return Some(String::new());
    }
    let with_slash = format!("{base_trim}/");
    from.strip_prefix(&with_slash).map(String::from)
}
