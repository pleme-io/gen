//! `gen fleet-migrate` — the typed tatara-lisp authoring surface over
//! the delta-only migration engine (`gen_cargo::fleet_migrate`).
//!
//! The fleet set + doctrine are authored declaratively as a
//! `.tatara.lisp` and compiled via `#[derive(TataraDomain)]` into a
//! typed `FleetMigrationPlan` — no brittle shell repo-list strings:
//!
//! ```lisp
//! (fleet-migration-plan
//!   :workspace-root "~/code/github/pleme-io"
//!   :exclude ("ishou" "tatara-lisp")
//!   :push #t)
//! ```
//!
//! (Booleans are Scheme-style `#t` / `#f`.)

use std::path::PathBuf;

use gen_cargo::fleet_migrate::{self, MigrateOpts, MigrateReport};
use serde::{Deserialize, Serialize};
use tatara_lisp::{compile_typed, DeriveTataraDomain};

/// A fleet delta-only migration plan. Field names map to kebab-case
/// kwargs (`workspace_root` → `:workspace-root`); `#[serde(default)]`
/// fields are optional.
#[derive(DeriveTataraDomain, Clone, Debug, Serialize, Deserialize)]
#[tatara(keyword = "fleet-migration-plan")]
pub struct FleetMigrationPlan {
    /// Directory whose immediate subdirectories are candidate repos.
    pub workspace_root: String,
    /// Explicit repo names to migrate. Empty → discover every eligible
    /// repo under `workspace_root` (has `Cargo.toml` + a committed
    /// `Cargo.build-spec.json` or `Cargo.gen.lock`).
    #[serde(default)]
    pub repos: Vec<String>,
    /// Repo names to exclude from discovery.
    #[serde(default)]
    pub exclude: Vec<String>,
    /// Push (fetch + ff-only + SHA-verify) after committing.
    #[serde(default)]
    pub push: bool,
    /// Commit as `gen-spec-bot` rather than the ambient git author.
    #[serde(default)]
    pub bot_identity: bool,
}

impl FleetMigrationPlan {
    /// Parse the first `(fleet-migration-plan …)` form from Lisp source.
    pub fn from_source(src: &str) -> Result<Self, String> {
        compile_typed::<FleetMigrationPlan>(src)
            .map_err(|e| e.to_string())?
            .into_iter()
            .next()
            .ok_or_else(|| "no (fleet-migration-plan …) form found".to_string())
    }

    /// Resolve the ordered list of repo paths this plan targets.
    pub fn resolve_repos(&self) -> Result<Vec<PathBuf>, String> {
        let root = expand_tilde(&self.workspace_root);
        // Explicit list wins (still honoring excludes).
        if !self.repos.is_empty() {
            return Ok(self
                .repos
                .iter()
                .filter(|n| !self.exclude.contains(n))
                .map(|n| root.join(n))
                .collect());
        }
        // Discovery: eligible = participating Rust repo (Cargo.toml + .git +
        // a build-spec or delta already present). The engine classifies the
        // rest (dirty / lock-mutated / no-delta) per repo.
        let mut dirs: Vec<PathBuf> = std::fs::read_dir(&root)
            .map_err(|e| format!("read_dir {}: {e}", root.display()))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_dir())
            .collect();
        dirs.sort();
        Ok(dirs
            .into_iter()
            .filter(|d| {
                let name = d
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                !name.starts_with('.')
                    && !self.exclude.contains(&name)
                    && d.join("Cargo.toml").exists()
                    && d.join(".git").exists()
                    && (d.join("Cargo.build-spec.json").exists() || d.join("Cargo.gen.lock").exists())
            })
            .collect())
    }

    pub fn opts(&self, force_no_push: bool) -> MigrateOpts {
        MigrateOpts {
            push: self.push && !force_no_push,
            bot_identity: self.bot_identity,
        }
    }
}

fn expand_tilde(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(p)
}

/// Parse + execute a plan from Lisp source; returns the typed report and
/// the resolved repo count.
pub fn run_plan(src: &str, force_no_push: bool) -> Result<MigrateReport, String> {
    let plan = FleetMigrationPlan::from_source(src)?;
    let repos = plan.resolve_repos()?;
    fleet_migrate::run(&repos, plan.opts(force_no_push)).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_plan() {
        let p = FleetMigrationPlan::from_source(
            r#"(fleet-migration-plan :workspace-root "/tmp/ws")"#,
        )
        .expect("parse");
        assert_eq!(p.workspace_root, "/tmp/ws");
        assert!(p.repos.is_empty());
        assert!(!p.push);
        assert!(!p.bot_identity);
    }

    #[test]
    fn parses_full_plan_with_lists_and_bools() {
        let p = FleetMigrationPlan::from_source(
            r#"(fleet-migration-plan
                 :workspace-root "~/code/github/pleme-io"
                 :repos ("seibi" "frost")
                 :exclude ("ishou")
                 :push #t
                 :bot-identity #f)"#,
        )
        .expect("parse");
        assert_eq!(p.repos, vec!["seibi", "frost"]);
        assert_eq!(p.exclude, vec!["ishou"]);
        assert!(p.push);
        assert!(!p.bot_identity);
    }

    #[test]
    fn explicit_repos_honor_excludes() {
        let p = FleetMigrationPlan::from_source(
            r#"(fleet-migration-plan :workspace-root "/tmp/ws" :repos ("a" "b" "c") :exclude ("b"))"#,
        )
        .unwrap();
        let repos = p.resolve_repos().unwrap();
        let names: Vec<_> = repos
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["a", "c"]);
    }

    #[test]
    fn no_push_override() {
        let p = FleetMigrationPlan::from_source(
            r#"(fleet-migration-plan :workspace-root "/tmp" :push #t)"#,
        )
        .unwrap();
        assert!(p.opts(false).push);
        assert!(!p.opts(true).push); // force_no_push overrides
    }
}
