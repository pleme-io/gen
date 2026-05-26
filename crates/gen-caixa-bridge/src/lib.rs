//! `gen-caixa-bridge` — typed bridge between caixa and gen.
//!
//! Caixa knows about gen, not the other way around. A (defcaixa …)
//! block can declare a `:gen` slot:
//!
//! ```lisp
//! (defcaixa my-thing
//!   :kind :biblioteca
//!   :gen (:adapter :cargo
//!         :render-mode :per-crate
//!         :nix-output "Cargo.nix"))
//! ```
//!
//! The caixa renderer constructs a typed [`CaixaToGen`] value from
//! that slot + the source root, calls [`CaixaToGen::route`], and acts
//! on the returned [`AdapterRoute`].

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use gen_config::RenderMode;

/// Caixa kinds — copy of the typed enum that lives in the caixa crate
/// (avoiding a cyclic dep). Kept in sync via a fleet test in M9b.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CaixaKind {
    /// Rust library (crates.io publish target).
    Biblioteca,
    /// Rust binary (single CLI / tool).
    Binario,
    /// Rust service (long-running daemon).
    Servico,
    /// Rust supervisor (multi-process orchestrator).
    Supervisor,
    /// Application (mixed-language: Rust + maybe Yew/wasm + maybe
    /// Ruby tooling + helm chart). Routes via polyglot.
    Aplicacao,
}

/// Input the caixa renderer hands to the bridge.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaixaToGen {
    pub caixa_kind: CaixaKind,
    pub source_root: PathBuf,
    /// Operator override; if set, bypasses the default adapter
    /// selection derived from `caixa_kind`.
    pub force_adapter: Option<String>,
    /// Render mode — usually carried over from caixa's `:build` slot.
    pub render_mode: RenderMode,
    /// Output path for the rendered Nix file (relative to
    /// `source_root`).
    pub nix_output: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AdapterRoute {
    /// One adapter dispatch (Biblioteca / Binario / Servico / Supervisor).
    Single {
        adapter: String,
        render_mode: RenderMode,
        nix_output: PathBuf,
    },
    /// Polyglot dispatch (Aplicacao kind) — multiple adapters land
    /// distinct output files.
    Polyglot {
        adapters: Vec<String>,
        render_mode: RenderMode,
        outputs: indexmap::IndexMap<String, PathBuf>,
    },
}

#[derive(Debug, Error)]
pub enum CaixaBridgeError {
    #[error("caixa source root does not exist: {0}")]
    MissingSourceRoot(PathBuf),
    #[error("nix output path must be relative to the source root, got `{0}`")]
    AbsoluteOutputPath(String),
}

impl CaixaToGen {
    pub fn route(&self) -> Result<AdapterRoute, CaixaBridgeError> {
        if !self.source_root.exists() {
            return Err(CaixaBridgeError::MissingSourceRoot(self.source_root.clone()));
        }
        if PathBuf::from(&self.nix_output).is_absolute() {
            return Err(CaixaBridgeError::AbsoluteOutputPath(self.nix_output.clone()));
        }
        // Operator override wins.
        if let Some(force) = &self.force_adapter {
            return Ok(AdapterRoute::Single {
                adapter: force.clone(),
                render_mode: self.render_mode,
                nix_output: self.source_root.join(&self.nix_output),
            });
        }
        match self.caixa_kind {
            CaixaKind::Biblioteca
            | CaixaKind::Binario
            | CaixaKind::Servico
            | CaixaKind::Supervisor => Ok(AdapterRoute::Single {
                adapter: "cargo".to_string(),
                render_mode: self.render_mode,
                nix_output: self.source_root.join(&self.nix_output),
            }),
            CaixaKind::Aplicacao => {
                // Probe for every adapter marker that's present and
                // route each to its own output file.
                let mut outputs: indexmap::IndexMap<String, PathBuf> = indexmap::IndexMap::new();
                let mut adapters: Vec<String> = Vec::new();
                let probe = [
                    ("Cargo.toml", "cargo", "Cargo.nix"),
                    ("package.json", "npm", "package-lock.nix"),
                    ("Gemfile", "bundler", "Gemfile.nix"),
                    ("pyproject.toml", "pip", "pyproject.nix"),
                    ("go.mod", "gomod", "go.nix"),
                ];
                for (marker, adapter, out) in probe {
                    if self.source_root.join(marker).exists() {
                        adapters.push(adapter.to_string());
                        outputs.insert(adapter.to_string(), self.source_root.join(out));
                    }
                }
                Ok(AdapterRoute::Polyglot {
                    adapters,
                    render_mode: self.render_mode,
                    outputs,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tempdir_with(files: &[&str]) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "gen-caixa-bridge-test-{}-{}",
            std::process::id(),
            n
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        for f in files {
            fs::write(dir.join(f), "").unwrap();
        }
        dir
    }

    #[test]
    fn biblioteca_routes_to_cargo() {
        let dir = tempdir_with(&["Cargo.toml"]);
        let req = CaixaToGen {
            caixa_kind: CaixaKind::Biblioteca,
            source_root: dir.clone(),
            force_adapter: None,
            render_mode: RenderMode::PerCrate,
            nix_output: "Cargo.nix".to_string(),
        };
        match req.route().unwrap() {
            AdapterRoute::Single { adapter, .. } => assert_eq!(adapter, "cargo"),
            other => panic!("expected Single, got {other:?}"),
        }
    }

    #[test]
    fn aplicacao_with_cargo_only_returns_polyglot_with_one_entry() {
        let dir = tempdir_with(&["Cargo.toml"]);
        let req = CaixaToGen {
            caixa_kind: CaixaKind::Aplicacao,
            source_root: dir.clone(),
            force_adapter: None,
            render_mode: RenderMode::PerCrate,
            nix_output: "ignored".to_string(),
        };
        match req.route().unwrap() {
            AdapterRoute::Polyglot { adapters, .. } => {
                assert_eq!(adapters, vec!["cargo".to_string()]);
            }
            other => panic!("expected Polyglot, got {other:?}"),
        }
    }

    #[test]
    fn aplicacao_with_three_languages_returns_three_adapter_entries() {
        let dir = tempdir_with(&["Cargo.toml", "package.json", "Gemfile"]);
        let req = CaixaToGen {
            caixa_kind: CaixaKind::Aplicacao,
            source_root: dir.clone(),
            force_adapter: None,
            render_mode: RenderMode::PerTree,
            nix_output: "ignored".to_string(),
        };
        match req.route().unwrap() {
            AdapterRoute::Polyglot {
                adapters, outputs, ..
            } => {
                assert_eq!(adapters.len(), 3);
                assert!(adapters.contains(&"cargo".to_string()));
                assert!(adapters.contains(&"npm".to_string()));
                assert!(adapters.contains(&"bundler".to_string()));
                assert_eq!(outputs.len(), 3);
            }
            other => panic!("expected Polyglot, got {other:?}"),
        }
    }

    #[test]
    fn force_adapter_overrides_caixa_kind() {
        let dir = tempdir_with(&["package.json"]);
        let req = CaixaToGen {
            caixa_kind: CaixaKind::Biblioteca,
            source_root: dir.clone(),
            force_adapter: Some("npm".to_string()),
            render_mode: RenderMode::PerCrate,
            nix_output: "Cargo.nix".to_string(),
        };
        match req.route().unwrap() {
            AdapterRoute::Single { adapter, .. } => assert_eq!(adapter, "npm"),
            other => panic!("expected Single, got {other:?}"),
        }
    }

    #[test]
    fn absolute_output_path_errors() {
        let dir = tempdir_with(&["Cargo.toml"]);
        let req = CaixaToGen {
            caixa_kind: CaixaKind::Biblioteca,
            source_root: dir.clone(),
            force_adapter: None,
            render_mode: RenderMode::PerCrate,
            nix_output: "/abs/path".to_string(),
        };
        assert!(matches!(
            req.route().unwrap_err(),
            CaixaBridgeError::AbsoluteOutputPath(_)
        ));
    }

    #[test]
    fn missing_source_root_errors() {
        let req = CaixaToGen {
            caixa_kind: CaixaKind::Biblioteca,
            source_root: PathBuf::from("/nonexistent/path"),
            force_adapter: None,
            render_mode: RenderMode::PerCrate,
            nix_output: "Cargo.nix".to_string(),
        };
        assert!(matches!(
            req.route().unwrap_err(),
            CaixaBridgeError::MissingSourceRoot(_)
        ));
    }
}
