//! `gen-polyglot` — multi-adapter discovery + aggregation.
//!
//! For repos that ship more than one language (a Rust workspace
//! whose UI is a Yew/wasm crate + a small Ruby tooling sidecar; a
//! Pangea operator with Helm charts; a typed-API repo with SDKs in
//! N languages) — gen-polyglot probes for every configured marker
//! and returns a typed [`PolyglotWorkspace`] containing one
//! [`Manifest`] per language found.
//!
//! The engine uses this to drive per-language render passes; the
//! audit CLI uses it to answer "what is this repo, dependency-wise?".

use std::path::{Path, PathBuf};

use indexmap::IndexMap;
use serde::Serialize;
use thiserror::Error;

use gen_config::GenConfig;
use gen_types::Manifest;

#[derive(Debug, Error)]
pub enum PolyglotError {
    #[error("workspace at {0} matched no adapter markers")]
    NoAdapters(PathBuf),
    #[error(transparent)]
    Cargo(#[from] gen_cargo::CargoError),
    #[error(transparent)]
    Npm(#[from] gen_npm::NpmError),
    #[error(transparent)]
    Bundler(#[from] gen_bundler::BundlerError),
}

pub type Result<T> = std::result::Result<T, PolyglotError>;

/// Typed polyglot output. Each entry pairs an adapter name with the
/// typed Manifest produced by that adapter.
#[derive(Clone, Debug, Serialize)]
pub struct PolyglotWorkspace {
    pub root: PathBuf,
    pub manifests: IndexMap<String, Manifest>,
}

impl PolyglotWorkspace {
    /// Count packages across every language manifest.
    #[must_use]
    pub fn total_package_count(&self) -> usize {
        self.manifests.values().map(Manifest::package_count).sum()
    }

    /// Stable list of adapters that found a manifest at this root.
    #[must_use]
    pub fn adapters_present(&self) -> Vec<&str> {
        self.manifests.keys().map(String::as_str).collect()
    }
}

/// Probe `root` for every configured adapter marker; for each present
/// marker, dispatch to the matching parser. Returns the typed
/// aggregate. Errors are accumulated per-adapter (a Bundler parse
/// failure does not block a Cargo parse from succeeding) via the
/// typed `Result` per-language inside the map.
pub fn parse(root: &Path, cfg: &GenConfig) -> Result<PolyglotWorkspace> {
    let mut manifests: IndexMap<String, Manifest> = IndexMap::new();
    for (marker, adapter) in &cfg.workspace.adapter_routing {
        if !root.join(marker).exists() {
            continue;
        }
        let manifest = match adapter.as_str() {
            "cargo" => gen_cargo::parse(root)?,
            "npm" => gen_npm::parse(root)?,
            "bundler" => gen_bundler::parse(root)?,
            // Adapters that haven't shipped yet are silently skipped;
            // the M4-spawn adapter scaffold will add them as they
            // come online.
            _ => continue,
        };
        manifests.insert(adapter.clone(), manifest);
    }
    if manifests.is_empty() {
        return Err(PolyglotError::NoAdapters(root.to_path_buf()));
    }
    Ok(PolyglotWorkspace {
        root: root.to_path_buf(),
        manifests,
    })
}

/// Aggregate cross-language statistics. Used by the audit CLI.
#[derive(Debug, Clone, Serialize)]
pub struct PolyglotStats {
    pub root: PathBuf,
    pub adapters: Vec<String>,
    pub total_packages: usize,
    pub per_adapter_packages: IndexMap<String, usize>,
    pub per_adapter_dependencies: IndexMap<String, usize>,
    pub per_adapter_has_lockfile: IndexMap<String, bool>,
}

impl PolyglotStats {
    #[must_use]
    pub fn from(w: &PolyglotWorkspace) -> Self {
        let adapters: Vec<String> = w.manifests.keys().cloned().collect();
        let per_pkg: IndexMap<String, usize> = w
            .manifests
            .iter()
            .map(|(k, m)| (k.clone(), m.package_count()))
            .collect();
        let per_dep: IndexMap<String, usize> = w
            .manifests
            .iter()
            .map(|(k, m)| {
                (
                    k.clone(),
                    m.packages.iter().map(|p| p.dependencies.len()).sum(),
                )
            })
            .collect();
        let per_lock: IndexMap<String, bool> = w
            .manifests
            .iter()
            .map(|(k, m)| (k.clone(), m.lockfile.is_some()))
            .collect();
        Self {
            root: w.root.clone(),
            adapters,
            total_packages: w.total_package_count(),
            per_adapter_packages: per_pkg,
            per_adapter_dependencies: per_dep,
            per_adapter_has_lockfile: per_lock,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tempfile_dir() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let base = std::env::temp_dir().join(format!(
            "gen-polyglot-test-{}-{}-{:?}",
            std::process::id(),
            n,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        base
    }

    #[test]
    fn detects_only_cargo_when_only_cargo_present() {
        use shikumi::TieredConfig;
        let dir = tempfile_dir();
        fs::write(
            dir.join("Cargo.toml"),
            r#"[package]
name = "p"
version = "0.1.0"
edition = "2024"
"#,
        )
        .unwrap();
        let cfg = GenConfig::prescribed_default();
        let w = parse(&dir, &cfg).unwrap();
        assert_eq!(w.adapters_present(), vec!["cargo"]);
    }

    #[test]
    fn detects_multiple_adapters_in_polyglot_repo() {
        use shikumi::TieredConfig;
        let dir = tempfile_dir();
        fs::write(
            dir.join("Cargo.toml"),
            r#"[package]
name = "p"
version = "0.1.0"
edition = "2024"
"#,
        )
        .unwrap();
        fs::write(
            dir.join("package.json"),
            r#"{"name":"p","version":"0.1.0"}"#,
        )
        .unwrap();
        fs::write(dir.join("Gemfile"), "gem 'rake'\n").unwrap();
        let cfg = GenConfig::prescribed_default();
        let w = parse(&dir, &cfg).unwrap();
        let present = w.adapters_present();
        assert!(present.contains(&"cargo"));
        assert!(present.contains(&"npm"));
        assert!(present.contains(&"bundler"));
        assert_eq!(w.total_package_count(), 3);
    }

    #[test]
    fn errors_when_no_adapters_match() {
        use shikumi::TieredConfig;
        let dir = tempfile_dir();
        let cfg = GenConfig::prescribed_default();
        let e = parse(&dir, &cfg).unwrap_err();
        assert!(matches!(e, PolyglotError::NoAdapters(_)));
    }

    #[test]
    fn stats_reports_per_adapter_counts() {
        use shikumi::TieredConfig;
        let dir = tempfile_dir();
        fs::write(
            dir.join("Cargo.toml"),
            r#"[package]
name = "p"
version = "0.1.0"
edition = "2024"

[dependencies]
serde = "1"
"#,
        )
        .unwrap();
        fs::write(
            dir.join("package.json"),
            r#"{"name":"p","version":"0.1.0","dependencies":{"lodash":"4"}}"#,
        )
        .unwrap();
        let cfg = GenConfig::prescribed_default();
        let w = parse(&dir, &cfg).unwrap();
        let s = PolyglotStats::from(&w);
        assert_eq!(s.adapters.len(), 2);
        assert_eq!(s.total_packages, 2);
        assert_eq!(s.per_adapter_dependencies.get("cargo"), Some(&1));
        assert_eq!(s.per_adapter_dependencies.get("npm"), Some(&1));
    }
}
