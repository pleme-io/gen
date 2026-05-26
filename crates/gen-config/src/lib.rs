//! `gen-config` — typed operator-facing configuration for the gen
//! engine. Implements [`shikumi::TieredConfig`]; every consumer
//! resolves via `GenConfig::resolve_from_env("GEN_TIER")` at startup.
//!
//! The four typed slot groups (workspace / cache / render / target)
//! cover every knob an operator can flip without recompiling. New
//! knobs land here, not in CLI flags — flags are a view, config is
//! the substrate.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use shikumi::TieredConfig;

/// Top-level config. Composes the four typed sub-slots.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenConfig {
    pub workspace: WorkspaceConfig,
    pub cache: CacheConfig,
    pub render: RenderConfig,
    pub target: TargetConfig,
}

impl Default for GenConfig {
    fn default() -> Self {
        Self::prescribed_default()
    }
}

/// Workspace discovery + adapter routing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    /// Root path the engine operates against. Empty = CWD.
    pub root: String,
    /// `(filename → adapter)` table. Engine probes the root for each
    /// file in declaration order and dispatches to the matching
    /// adapter. New adapters extend the map; consumers never touch
    /// the dispatch logic.
    pub adapter_routing: IndexMap<String, String>,
    /// Override the auto-detected adapter — `cargo` / `npm` / `bundler`
    /// / `pip` / `gomod` / `helm` / `auto` (default).
    pub force_adapter: Option<String>,
}

/// Substituter / cache settings. Engine consults this before deciding
/// to evaluate a derivation; substituter hits short-circuit the rebuild.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheConfig {
    /// Substituter URLs (binary-cache backends). The engine consults
    /// each in order; first hit wins. Empty list disables substituter
    /// lookup (always-build mode, useful for CI's first canary).
    pub substituters: Vec<String>,
    /// Public keys trusted for substituter signatures.
    pub trusted_public_keys: Vec<String>,
    /// Build the package even when a substituter hit is found. Useful
    /// for cache-population runs.
    pub always_build: bool,
}

/// Render-shape settings. Controls whether the engine emits per-crate
/// derivations (crate2nix shape, default for incremental local dev) or
/// per-tree derivations (crane shape, default for CI bulk builds).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderConfig {
    pub mode: RenderMode,
    /// Optional output path for the rendered Nix expression. Empty =
    /// stdout (most operator workflows).
    pub output_path: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RenderMode {
    /// Per-crate derivation. Best for incremental local dev — touch one
    /// crate, rebuild only that crate's closure.
    PerCrate,
    /// Per-tree derivation. Best for CI's bulk builds — fewer
    /// derivations to evaluate, no per-edge fan-out cost.
    PerTree,
}

/// Concrete target the engine evaluates predicates against.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetConfig {
    pub os: String,
    pub cpu: String,
    pub libc: Option<String>,
}

impl TieredConfig for GenConfig {
    fn bare() -> Self {
        Self {
            workspace: WorkspaceConfig {
                root: String::new(),
                adapter_routing: IndexMap::new(),
                force_adapter: None,
            },
            cache: CacheConfig {
                substituters: Vec::new(),
                trusted_public_keys: Vec::new(),
                always_build: false,
            },
            render: RenderConfig {
                mode: RenderMode::PerCrate,
                output_path: String::new(),
            },
            target: TargetConfig {
                os: String::new(),
                cpu: String::new(),
                libc: None,
            },
        }
    }

    fn prescribed_default() -> Self {
        let mut adapter_routing = IndexMap::new();
        adapter_routing.insert("Cargo.toml".to_string(), "cargo".to_string());
        adapter_routing.insert("package.json".to_string(), "npm".to_string());
        adapter_routing.insert("Gemfile".to_string(), "bundler".to_string());
        adapter_routing.insert("pyproject.toml".to_string(), "pip".to_string());
        adapter_routing.insert("go.mod".to_string(), "gomod".to_string());
        adapter_routing.insert("Chart.yaml".to_string(), "helm".to_string());

        Self {
            workspace: WorkspaceConfig {
                root: ".".to_string(),
                adapter_routing,
                force_adapter: None,
            },
            cache: CacheConfig {
                substituters: vec![
                    "https://cache.nixos.org".to_string(),
                    "http://cache.plo.quero.cloud/nexus".to_string(),
                ],
                trusted_public_keys: vec![
                    "cache.nixos.org-1:6NCHdD59X431o0gWypbMrAURkbJ16ZPMQFGspcDShjY=".to_string(),
                ],
                always_build: false,
            },
            render: RenderConfig {
                mode: RenderMode::PerCrate,
                output_path: String::new(),
            },
            target: TargetConfig {
                os: host_os(),
                cpu: host_cpu(),
                libc: host_libc(),
            },
        }
    }

    fn discovered() -> Self {
        let mut c = Self::bare();
        c.target = TargetConfig {
            os: host_os(),
            cpu: host_cpu(),
            libc: host_libc(),
        };
        if let Ok(cwd) = std::env::current_dir() {
            c.workspace.root = cwd.display().to_string();
        }
        c
    }
}

fn host_os() -> String {
    #[cfg(target_os = "linux")]
    {
        "linux".to_string()
    }
    #[cfg(target_os = "macos")]
    {
        "macos".to_string()
    }
    #[cfg(target_os = "windows")]
    {
        "windows".to_string()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        "unknown".to_string()
    }
}

fn host_cpu() -> String {
    #[cfg(target_arch = "x86_64")]
    {
        "x86_64".to_string()
    }
    #[cfg(target_arch = "aarch64")]
    {
        "aarch64".to_string()
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        "unknown".to_string()
    }
}

fn host_libc() -> Option<String> {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        Some("gnu".to_string())
    }
    #[cfg(all(target_os = "linux", target_env = "musl"))]
    {
        Some("musl".to_string())
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
    #[cfg(all(target_os = "linux", not(any(target_env = "gnu", target_env = "musl"))))]
    {
        None
    }
}

/// Convenience: build a [`gen_types::Target`] from the config's
/// [`TargetConfig`]. Engines call this to seed predicate evaluation.
#[must_use]
pub fn target_from_config(t: &TargetConfig) -> gen_types::Target {
    gen_types::Target {
        os: t.os.clone(),
        cpu: t.cpu.clone(),
        libc: t.libc.clone(),
        engines: indexmap::IndexMap::new(),
        python_env_markers: indexmap::IndexMap::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_is_empty() {
        let c = GenConfig::bare();
        assert!(c.workspace.root.is_empty());
        assert!(c.workspace.adapter_routing.is_empty());
        assert!(c.cache.substituters.is_empty());
        assert!(c.target.os.is_empty());
    }

    #[test]
    fn prescribed_default_has_fleet_substituters() {
        let c = GenConfig::prescribed_default();
        assert!(c.cache.substituters.iter().any(|s| s.contains("cache.nixos.org")));
        assert!(c.cache.substituters.iter().any(|s| s.contains("plo.quero.cloud")));
    }

    #[test]
    fn prescribed_default_routes_cargo() {
        let c = GenConfig::prescribed_default();
        assert_eq!(
            c.workspace.adapter_routing.get("Cargo.toml").map(String::as_str),
            Some("cargo")
        );
    }

    #[test]
    fn discovered_seeds_target_from_host() {
        let c = GenConfig::discovered();
        assert!(matches!(c.target.os.as_str(), "linux" | "macos" | "windows" | "unknown"));
        assert!(matches!(c.target.cpu.as_str(), "x86_64" | "aarch64" | "unknown"));
    }

    #[test]
    fn round_trips_through_serde_yaml() {
        let c = GenConfig::prescribed_default();
        let y = serde_yaml::to_string(&c).unwrap();
        let parsed: GenConfig = serde_yaml::from_str(&y).unwrap();
        assert_eq!(c, parsed);
    }

    #[test]
    fn target_from_config_round_trips() {
        let c = GenConfig::prescribed_default();
        let t = target_from_config(&c.target);
        assert_eq!(t.os, c.target.os);
        assert_eq!(t.cpu, c.target.cpu);
    }
}
