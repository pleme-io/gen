//! `gen fleet-migrate` — DELTA-ONLY fleet migration engine.
//!
//! Converts a Rust repo to the delta-only doctrine: regenerate the spec,
//! commit the slim `Cargo.gen.lock`, and retire (`git rm --cached` +
//! gitignore) the full `Cargo.build-spec.json`. substrate's
//! lockfile-builder reconstructs the spec in pure Nix from `Cargo.lock`
//! + the delta (delta > build-spec > IFD), so the big artifact is
//! redundant operator-surface noise.
//!
//! This is the typed Rust replacement for the fragile shell sweep that
//! drove the initial rollout. Every sharp edge that bit that sweep is
//! handled structurally here:
//!   - NEVER `git add -A` — staging is path-targeted, so leaked build
//!     artifacts / unrelated working-tree files are never committed.
//!   - Lock-mutation guard: a repo whose `gen build` re-resolves
//!     `Cargo.lock` has an unpatched transitive git self-reference (the
//!     gen / ishou class). Migrating it would commit a surprise lock
//!     change, so it is skipped + restored, not committed.
//!   - No-delta guard: a single-target / degenerate build emits no
//!     `Cargo.gen.lock`. Retiring build-spec there would strand the repo
//!     on IFD, so the migration no-ops instead.
//!   - Divergence-aware push: fetch + fast-forward-only before pushing;
//!     a genuinely diverged remote yields a typed failure, never a force.
//!   - Verification by branch SHA from `ls-remote --heads` — immune to
//!     repos with an ambiguous tag+branch of the same name.
//!
//! Every shell-level error maps to a typed `MigrateOutcome` variant; no
//! opaque string reaches the operator, and no `PASS`-echo can lie about
//! a commit that did not land.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::error::CargoError;

const BUILD_SPEC: &str = "Cargo.build-spec.json";
const DELTA: &str = "Cargo.gen.lock";
/// Paths the migration is allowed to touch. A tracked change to any
/// other path makes the repo "blocking-dirty" (someone else's WIP).
const ALLOWED: [&str; 3] = [BUILD_SPEC, DELTA, ".gitignore"];

/// One repo's migration outcome. Serializes to a kebab-case `status`
/// tag so the CLI can emit a typed JSON report.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum MigrateOutcome {
    /// Migrated: delta committed, build-spec retired (and pushed if asked).
    Migrated {
        commit_sha: String,
        pushed: bool,
        build_spec_retired: bool,
        elapsed_ms: u64,
    },
    /// Already delta-only with a fresh delta — nothing to do (idempotent).
    AlreadyDeltaOnly,
    /// No `Cargo.toml` + `Cargo.lock` — not a buildable Rust repo.
    SkippedNotRust,
    /// No `.git` directory.
    SkippedNotAGitRepo,
    /// Tracked working-tree changes outside the migration file set.
    SkippedDirty { detail: String },
    /// `gen build` re-resolved `Cargo.lock` (unpatched git self-reference).
    SkippedLockMutated,
    /// `gen build` emitted no delta (single-target / degenerate spec).
    SkippedNoDelta,
    /// A typed structural failure.
    Failed {
        category: MigrateFailure,
        detail: String,
        elapsed_ms: u64,
    },
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum MigrateFailure {
    /// `generate_multi_target_and_write` returned an error.
    BuildFailed,
    /// `git status` / inspection failed.
    GitInspectionFailed,
    /// `git rm` / `git add` failed.
    GitStageFailed,
    /// `git commit` failed.
    GitCommitFailed,
    /// Remote diverged (non-fast-forward); not force-pushed.
    PushDiverged,
    /// `git push` failed for another reason.
    PushFailed,
    /// Post-push remote branch SHA != local HEAD.
    VerifyMismatch,
}

/// Migration options.
#[derive(Clone, Copy, Debug)]
pub struct MigrateOpts {
    /// Push (with fetch + ff-only) after committing.
    pub push: bool,
    /// Commit as `gen-spec-bot` (vs the ambient git author).
    pub bot_identity: bool,
}

/// Aggregate report over a repo set.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MigrateReport {
    pub outcomes: IndexMap<String, MigrateOutcome>,
    pub total_elapsed_ms: u64,
}

impl MigrateReport {
    pub fn total(&self) -> usize {
        self.outcomes.len()
    }
    pub fn migrated_count(&self) -> usize {
        self.outcomes
            .values()
            .filter(|o| matches!(o, MigrateOutcome::Migrated { .. }))
            .count()
    }
    pub fn pushed_count(&self) -> usize {
        self.outcomes
            .values()
            .filter(|o| matches!(o, MigrateOutcome::Migrated { pushed: true, .. }))
            .count()
    }
    pub fn failed_count(&self) -> usize {
        self.outcomes
            .values()
            .filter(|o| matches!(o, MigrateOutcome::Failed { .. }))
            .count()
    }
    pub fn skipped_count(&self) -> usize {
        self.outcomes
            .values()
            .filter(|o| {
                matches!(
                    o,
                    MigrateOutcome::SkippedNotRust
                        | MigrateOutcome::SkippedNotAGitRepo
                        | MigrateOutcome::SkippedDirty { .. }
                        | MigrateOutcome::SkippedLockMutated
                        | MigrateOutcome::SkippedNoDelta
                        | MigrateOutcome::AlreadyDeltaOnly
                )
            })
            .count()
    }
}

/// Migrate every repo in `repos`, in the given order.
pub fn run(repos: &[PathBuf], opts: MigrateOpts) -> Result<MigrateReport, CargoError> {
    let started = Instant::now();
    let mut outcomes: IndexMap<String, MigrateOutcome> = IndexMap::new();
    for repo in repos {
        let name = repo
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| repo.display().to_string());
        outcomes.insert(name, migrate_one(repo, opts));
    }
    Ok(MigrateReport {
        outcomes,
        total_elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

/// Migrate a single repo to delta-only.
pub fn migrate_one(repo: &Path, opts: MigrateOpts) -> MigrateOutcome {
    let started = Instant::now();
    let ms = || started.elapsed().as_millis() as u64;

    if !repo.join(".git").exists() {
        return MigrateOutcome::SkippedNotAGitRepo;
    }
    if !(repo.join("Cargo.toml").exists() && repo.join("Cargo.lock").exists()) {
        return MigrateOutcome::SkippedNotRust;
    }

    // Blocking-dirty: any TRACKED change outside the migration file set.
    match blocking_dirty(repo) {
        Err(detail) => {
            return MigrateOutcome::Failed {
                category: MigrateFailure::GitInspectionFailed,
                detail,
                elapsed_ms: ms(),
            }
        }
        Ok(Some(detail)) => return MigrateOutcome::SkippedDirty { detail },
        Ok(None) => {}
    }

    // Sync to latest remote BEFORE building/committing so our commit lands
    // on top of any daemon flake.lock chores (avoids self-inflicted
    // divergence). Only when pushing; a dry run must not mutate the tree.
    if opts.push {
        if let Ok(branch) = current_branch(repo) {
            let _ = run_git(repo, &["fetch", "--quiet", "origin", &branch]);
            // ff-only: no-op when current, advances past chores, fails
            // (left untouched) on genuine divergence — handled at push.
            let _ = run_git(repo, &["merge", "--ff-only", &format!("origin/{branch}")]);
        }
    }

    // Build in-process (no subprocess / binary-path fragility). gen build
    // is read-only on Cargo.lock — we verify that below.
    let lock_before = std::fs::read(repo.join("Cargo.lock")).ok();
    if let Err(e) = crate::build_spec::generate_multi_target_and_write(repo) {
        return MigrateOutcome::Failed {
            category: MigrateFailure::BuildFailed,
            detail: e.to_string(),
            elapsed_ms: ms(),
        };
    }
    let lock_after = std::fs::read(repo.join("Cargo.lock")).ok();
    if lock_before != lock_after {
        // Unpatched git self-reference re-resolved the lock. Restore and
        // skip — never commit a surprise lock change.
        let _ = run_git(repo, &["checkout", "--", "Cargo.lock"]);
        if build_spec_tracked(repo) {
            let _ = run_git(repo, &["checkout", "--", BUILD_SPEC]);
        }
        let _ = remove_if_untracked(repo, DELTA);
        return MigrateOutcome::SkippedLockMutated;
    }

    // No delta → retiring build-spec would strand the repo on IFD.
    if !repo.join(DELTA).exists() {
        if build_spec_tracked(repo) {
            let _ = run_git(repo, &["checkout", "--", BUILD_SPEC]);
        }
        return MigrateOutcome::SkippedNoDelta;
    }

    // Retire build-spec: untrack if tracked, ensure gitignored.
    let mut retired = false;
    if build_spec_tracked(repo) {
        if let Err(e) = run_git(repo, &["rm", "--quiet", "--cached", "--", BUILD_SPEC]) {
            return MigrateOutcome::Failed {
                category: MigrateFailure::GitStageFailed,
                detail: e,
                elapsed_ms: ms(),
            };
        }
        retired = true;
    }
    if let Err(e) = ensure_gitignored(repo, BUILD_SPEC) {
        return MigrateOutcome::Failed {
            category: MigrateFailure::GitStageFailed,
            detail: e,
            elapsed_ms: ms(),
        };
    }

    // Stage the delta + the .gitignore (path-targeted — never `-A`).
    if let Err(e) = run_git(repo, &["add", "--", DELTA, ".gitignore"]) {
        return MigrateOutcome::Failed {
            category: MigrateFailure::GitStageFailed,
            detail: e,
            elapsed_ms: ms(),
        };
    }

    // Nothing staged → already delta-only with an unchanged delta.
    // `git diff --cached --quiet` exits 0 (Ok) when there is no staged diff.
    if run_git(repo, &["diff", "--cached", "--quiet"]).is_ok() {
        return MigrateOutcome::AlreadyDeltaOnly;
    }

    // Commit.
    let msg = "spec: retire committed Cargo.build-spec.json — delta-only\n\n\
        Drop the full build-spec from version control + gitignore it; the slim\n\
        Cargo.gen.lock delta is the sole committed spec source. substrate's\n\
        lockfile-builder reconstructs the build-spec in pure Nix from Cargo.lock\n\
        + the delta (delta > build-spec > IFD), so the big artifact is redundant\n\
        operator-surface noise. Cargo.lock unchanged (gen build is read-only).\n\n\
        Migrated by `gen fleet-migrate`.";
    let mut commit_args: Vec<&str> = Vec::new();
    if opts.bot_identity {
        commit_args.extend_from_slice(&[
            "-c",
            "user.name=gen-spec-bot",
            "-c",
            "user.email=gen-spec-bot@pleme-io.invalid",
        ]);
    }
    commit_args.extend_from_slice(&["commit", "--quiet", "-m", msg]);
    if let Err(e) = run_git(repo, &commit_args) {
        return MigrateOutcome::Failed {
            category: MigrateFailure::GitCommitFailed,
            detail: e,
            elapsed_ms: ms(),
        };
    }
    let sha = run_git(repo, &["rev-parse", "HEAD"])
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    // Push (divergence-aware) + verify by branch SHA.
    let mut pushed = false;
    if opts.push {
        let branch = current_branch(repo).unwrap_or_else(|_| "main".to_string());
        match run_git(repo, &["push", "origin", &branch]) {
            Ok(_) => {}
            Err(e) => {
                let category = if e.contains("non-fast-forward") || e.contains("rejected") {
                    MigrateFailure::PushDiverged
                } else {
                    MigrateFailure::PushFailed
                };
                return MigrateOutcome::Failed {
                    category,
                    detail: e,
                    elapsed_ms: ms(),
                };
            }
        }
        // Authoritative: the branch SHA from ls-remote --heads (never a tag).
        let remote = run_git(repo, &["ls-remote", "--heads", "origin", &branch])
            .ok()
            .and_then(|s| s.split_whitespace().next().map(str::to_string))
            .unwrap_or_default();
        if remote != sha {
            return MigrateOutcome::Failed {
                category: MigrateFailure::VerifyMismatch,
                detail: format!("remote={remote} local={sha}"),
                elapsed_ms: ms(),
            };
        }
        pushed = true;
    }

    MigrateOutcome::Migrated {
        commit_sha: sha,
        pushed,
        build_spec_retired: retired,
        elapsed_ms: ms(),
    }
}

// ── helpers ─────────────────────────────────────────────────────────

/// `Some(detail)` if the working tree has a TRACKED change outside the
/// migration file set. Untracked files (`??`) never block — they cannot
/// be committed by our path-targeted staging.
fn blocking_dirty(repo: &Path) -> Result<Option<String>, String> {
    let out = run_git(repo, &["status", "--porcelain"])?;
    for line in out.lines() {
        if line.len() < 4 {
            continue;
        }
        let (status, rest) = line.split_at(2);
        if status == "??" {
            continue; // untracked — ignored
        }
        // Handle rename "old -> new": the committed path is the new one.
        let path = rest.trim();
        let path = path.rsplit(" -> ").next().unwrap_or(path).trim();
        if !ALLOWED.contains(&path) {
            return Ok(Some(format!("{status} {path}")));
        }
    }
    Ok(None)
}

fn build_spec_tracked(repo: &Path) -> bool {
    run_git(repo, &["ls-files", "--error-unmatch", BUILD_SPEC]).is_ok()
}

fn current_branch(repo: &Path) -> Result<String, String> {
    run_git(repo, &["rev-parse", "--abbrev-ref", "HEAD"]).map(|s| s.trim().to_string())
}

fn remove_if_untracked(repo: &Path, rel: &str) -> Result<(), String> {
    // Only delete the file if git does not track it (a transient artifact).
    if run_git(repo, &["ls-files", "--error-unmatch", rel]).is_err() {
        let _ = std::fs::remove_file(repo.join(rel));
    }
    Ok(())
}

/// Append `entry` to `<repo>/.gitignore` if not already present.
fn ensure_gitignored(repo: &Path, entry: &str) -> Result<(), String> {
    let gi = repo.join(".gitignore");
    let cur = std::fs::read_to_string(&gi).unwrap_or_default();
    if cur.lines().any(|l| l.trim() == entry) {
        return Ok(());
    }
    let mut new = cur;
    if !new.is_empty() && !new.ends_with('\n') {
        new.push('\n');
    }
    new.push_str(entry);
    new.push('\n');
    std::fs::write(&gi, new).map_err(|e| format!("write .gitignore: {e}"))
}

/// Run a git command in `repo`. Non-zero exit → `Err(stderr)`.
fn run_git(repo: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .map_err(|e| format!("git {args:?}: spawn failed: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "git {args:?} → exit {}: {}",
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_git_repo(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("genfm-{}-{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        run_git(&dir, &["init", "--quiet"]).unwrap();
        run_git(&dir, &["config", "user.email", "t@example.com"]).unwrap();
        run_git(&dir, &["config", "user.name", "t"]).unwrap();
        dir
    }

    fn write(dir: &Path, rel: &str, body: &str) {
        std::fs::write(dir.join(rel), body).unwrap();
    }

    #[test]
    fn ensure_gitignored_appends_and_is_idempotent() {
        let dir = temp_git_repo("gi");
        ensure_gitignored(&dir, BUILD_SPEC).unwrap();
        let after_first = std::fs::read_to_string(dir.join(".gitignore")).unwrap();
        assert!(after_first.lines().any(|l| l == BUILD_SPEC));
        // Idempotent: second call adds no duplicate.
        ensure_gitignored(&dir, BUILD_SPEC).unwrap();
        let after_second = std::fs::read_to_string(dir.join(".gitignore")).unwrap();
        assert_eq!(after_first, after_second);
        assert_eq!(
            after_second.lines().filter(|l| *l == BUILD_SPEC).count(),
            1
        );
    }

    #[test]
    fn ensure_gitignored_preserves_existing_entries() {
        let dir = temp_git_repo("gi2");
        write(&dir, ".gitignore", "/target\n");
        ensure_gitignored(&dir, BUILD_SPEC).unwrap();
        let gi = std::fs::read_to_string(dir.join(".gitignore")).unwrap();
        assert!(gi.contains("/target"));
        assert!(gi.lines().any(|l| l == BUILD_SPEC));
    }

    #[test]
    fn blocking_dirty_clean_tree_is_none() {
        let dir = temp_git_repo("clean");
        write(&dir, "foo.txt", "hi");
        run_git(&dir, &["add", "."]).unwrap();
        run_git(&dir, &["commit", "--quiet", "-m", "init"]).unwrap();
        assert_eq!(blocking_dirty(&dir).unwrap(), None);
    }

    #[test]
    fn blocking_dirty_flags_tracked_change_outside_allowlist() {
        let dir = temp_git_repo("dirty");
        write(&dir, "foo.txt", "hi");
        run_git(&dir, &["add", "."]).unwrap();
        run_git(&dir, &["commit", "--quiet", "-m", "init"]).unwrap();
        write(&dir, "foo.txt", "modified"); // tracked, non-allowed
        let blocked = blocking_dirty(&dir).unwrap();
        assert!(blocked.is_some(), "tracked non-allowed change must block");
        assert!(blocked.unwrap().contains("foo.txt"));
    }

    #[test]
    fn blocking_dirty_allows_migration_files_and_untracked() {
        let dir = temp_git_repo("allow");
        write(&dir, "foo.txt", "hi");
        write(&dir, BUILD_SPEC, "{}");
        run_git(&dir, &["add", "."]).unwrap();
        run_git(&dir, &["commit", "--quiet", "-m", "init"]).unwrap();
        // Tracked change to an ALLOWED migration file → not blocking.
        write(&dir, BUILD_SPEC, "{\"v\":10}");
        // Untracked random file → not blocking (can't be staged by us).
        write(&dir, "scratch.tmp", "x");
        assert_eq!(
            blocking_dirty(&dir).unwrap(),
            None,
            "allowed-file change + untracked file must not block"
        );
    }

    #[test]
    fn build_spec_tracked_reflects_git_state() {
        let dir = temp_git_repo("track");
        write(&dir, BUILD_SPEC, "{}");
        assert!(!build_spec_tracked(&dir), "untracked before add");
        run_git(&dir, &["add", "."]).unwrap();
        run_git(&dir, &["commit", "--quiet", "-m", "init"]).unwrap();
        assert!(build_spec_tracked(&dir), "tracked after commit");
    }

    #[test]
    fn migrate_one_non_rust_repo_skipped() {
        let dir = temp_git_repo("notrust");
        assert!(matches!(
            migrate_one(&dir, MigrateOpts { push: false, bot_identity: false }),
            MigrateOutcome::SkippedNotRust
        ));
    }

    #[test]
    fn migrate_one_non_git_skipped() {
        let dir = std::env::temp_dir().join(format!("genfm-{}-nogit", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        write(&dir, "Cargo.toml", "[package]");
        write(&dir, "Cargo.lock", "");
        assert!(matches!(
            migrate_one(&dir, MigrateOpts { push: false, bot_identity: false }),
            MigrateOutcome::SkippedNotAGitRepo
        ));
    }
}
