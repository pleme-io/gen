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
    /// Per-repo build timeout in seconds (default 300). A build that
    /// exceeds it — e.g. a stale git pin that hangs a fetch — is killed
    /// and the repo is reported `skipped-build-timeout`.
    #[serde(default)]
    pub build_timeout_secs: Option<i64>,
    /// Concurrent repos (default 1). Each repo is independent; scale to
    /// ~cpu cores. Overridable from the CLI with `--jobs`.
    #[serde(default)]
    pub jobs: Option<i64>,
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

    pub fn opts(&self, gen_bin: PathBuf, force_no_push: bool, jobs_override: Option<usize>) -> MigrateOpts {
        MigrateOpts {
            push: self.push && !force_no_push,
            bot_identity: self.bot_identity,
            gen_bin,
            build_timeout_secs: self.build_timeout_secs.unwrap_or(300).max(1) as u64,
            jobs: jobs_override.unwrap_or_else(|| self.jobs.unwrap_or(1).max(1) as usize),
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
pub fn run_plan(
    src: &str,
    force_no_push: bool,
    jobs_override: Option<usize>,
) -> Result<MigrateReport, String> {
    let plan = FleetMigrationPlan::from_source(src)?;
    let repos = plan.resolve_repos()?;
    // The build runs each repo in a killable subprocess of THIS binary.
    // STABILIZE the binary first: a fleet run may take many minutes, and a
    // concurrent `cargo build` of the gen workspace can unlink
    // target/debug/gen mid-run — every subsequent subprocess spawn then
    // fails with "No such file or directory". Copy the executable to a
    // temp path outside the cargo target (immune to that churn) and spawn
    // from there. Falls back to current_exe() if the copy fails.
    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    let stable = stabilize_binary(&exe);
    let gen_bin = stable.clone().unwrap_or(exe);
    let result = fleet_migrate::run(&repos, &plan.opts(gen_bin, force_no_push, jobs_override))
        .map_err(|e| e.to_string());
    if let Some(p) = stable {
        let _ = std::fs::remove_file(p);
    }
    result
}

/// Copy `exe` to a temp path (executable), returning it on success. The
/// copy survives a `cargo` rebuild that unlinks the original target/ binary.
fn stabilize_binary(exe: &std::path::Path) -> Option<PathBuf> {
    let dst = std::env::temp_dir().join(format!("gen-fleet-{}", std::process::id()));
    std::fs::copy(exe, &dst).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dst, std::fs::Permissions::from_mode(0o755)).ok()?;
    }
    Some(dst)
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
    fn stabilize_binary_copies_executable() {
        // Make a fake "binary", stabilize it, assert the copy exists, is
        // distinct from the source, and is executable.
        let src = std::env::temp_dir().join(format!("genfm-src-{}", std::process::id()));
        std::fs::write(&src, b"#!/bin/sh\ntrue\n").unwrap();
        let stable = stabilize_binary(&src).expect("stabilize");
        assert!(stable.exists());
        assert_ne!(stable, src);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&stable).unwrap().permissions().mode();
            assert!(mode & 0o111 != 0, "stable binary must be executable");
        }
        let _ = std::fs::remove_file(&src);
        let _ = std::fs::remove_file(&stable);
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
        let bin = PathBuf::from("/bin/true");
        assert!(p.opts(bin.clone(), false, None).push);
        assert!(!p.opts(bin.clone(), true, None).push); // force_no_push overrides
        assert_eq!(p.build_timeout_secs, None); // default applied at opts()
        assert_eq!(p.opts(bin.clone(), false, None).jobs, 1); // default
        assert_eq!(p.opts(bin, false, Some(12)).jobs, 12); // CLI override
    }
}
