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
/// Deprecated crate2nix-era sidecar; retired (like build-spec) by the
/// delta-only conversion.
const CRATE_HASHES: &str = "crate-hashes.json";

/// A dirty path that is a STALE BUILD ARTIFACT — safe to discard so the
/// conversion can proceed (vs real source/lock WIP, which blocks). Covers
/// committed `target/` trees and the deprecated `crate-hashes.json`.
fn is_discardable_artifact(path: &str) -> bool {
    path == CRATE_HASHES || path == "target" || path.starts_with("target/")
}

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
    /// `gen build` exceeded the timeout (e.g. a stale git pin that hangs
    /// a fetch). The build subprocess + its process group were killed.
    SkippedBuildTimeout,
    /// Local HEAD has commits not on the remote (ahead/diverged). Not
    /// reset — local work is preserved rather than destroyed.
    SkippedLocalAhead,
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
#[derive(Clone, Debug)]
pub struct MigrateOpts {
    /// Push (with fetch + ff-only) after committing.
    pub push: bool,
    /// Commit as `gen-spec-bot` (vs the ambient git author).
    pub bot_identity: bool,
    /// Path to the `gen` binary used to build each repo in a killable
    /// subprocess (typically the running executable, `current_exe()`).
    pub gen_bin: PathBuf,
    /// Per-repo build timeout in seconds. A build that exceeds it (e.g. a
    /// stale git pin that hangs a fetch) is killed → `SkippedBuildTimeout`.
    pub build_timeout_secs: u64,
    /// Number of repos migrated concurrently (>=1). Each repo is
    /// independent (own remote, own subprocess), so this scales nearly
    /// linearly; bound it to ~cpu cores.
    pub jobs: usize,
    /// Before building, `cargo update` the repo's pleme-io git deps to
    /// their live `main` HEADs — heals stale (GC'd-rev) `branch="main"`
    /// pins that would otherwise hang gen build's prefetch. Stages the
    /// refreshed Cargo.lock with the commit.
    pub refresh_git_deps: bool,
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
                        | MigrateOutcome::SkippedBuildTimeout
                        | MigrateOutcome::SkippedLocalAhead
                        | MigrateOutcome::AlreadyDeltaOnly
                )
            })
            .count()
    }
}

/// Migrate every repo in `repos`, with up to `opts.jobs` running
/// concurrently. Repos are independent (own remote, own build subprocess),
/// so concurrency scales nearly linearly; the shared cargo git/registry
/// cache is safe under cargo's own file locks. Outcomes are returned in
/// the input order regardless of completion order.
pub fn run(repos: &[PathBuf], opts: &MigrateOpts) -> Result<MigrateReport, CargoError> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    let started = Instant::now();
    let jobs = opts.jobs.max(1);
    let next = AtomicUsize::new(0);
    let collected: Mutex<Vec<(usize, String, MigrateOutcome)>> = Mutex::new(Vec::new());

    std::thread::scope(|scope| {
        for _ in 0..jobs {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::SeqCst);
                if i >= repos.len() {
                    break;
                }
                let repo = &repos[i];
                let name = repo
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| repo.display().to_string());
                let outcome = migrate_one(repo, opts);
                collected.lock().unwrap().push((i, name, outcome));
            });
        }
    });

    let mut rows = collected.into_inner().unwrap();
    rows.sort_by_key(|(i, _, _)| *i);
    let outcomes: IndexMap<String, MigrateOutcome> =
        rows.into_iter().map(|(_, n, o)| (n, o)).collect();
    Ok(MigrateReport {
        outcomes,
        total_elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

/// Migrate a single repo to delta-only.
pub fn migrate_one(repo: &Path, opts: &MigrateOpts) -> MigrateOutcome {
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

    // Only stale-artifact dirt (target/, crate-hashes.json) remains — discard
    // it so the tree is clean for sync + build. blocking_dirty already
    // excluded real source/lock WIP, so this is safe.
    discard_stale_artifacts(repo);

    // Sync to remote BEFORE building so the delta reflects the pushed
    // state (local clones may be stale, predating a fleet batch). Only
    // when pushing; a dry run must not mutate the tree. Safe by
    // construction: we only hard-reset when the tree is clean (checked
    // above) AND local HEAD is an ANCESTOR of the remote (i.e. merely
    // behind) — never when local has commits the remote lacks, which we
    // report as SkippedLocalAhead rather than destroy.
    if opts.push {
        if let Ok(branch) = current_branch(repo) {
            let _ = run_git(repo, &["fetch", "--quiet", "origin", &branch]);
            let remote_ref = format!("origin/{branch}");
            let head = run_git(repo, &["rev-parse", "HEAD"]).unwrap_or_default();
            let remote = run_git(repo, &["rev-parse", &remote_ref]).unwrap_or_default();
            if !remote.is_empty() && head.trim() != remote.trim() {
                // is local an ancestor of remote (behind, clean to ff)?
                if run_git(repo, &["merge-base", "--is-ancestor", "HEAD", &remote_ref]).is_ok() {
                    let _ = run_git(repo, &["reset", "--hard", &remote_ref]);
                } else {
                    return MigrateOutcome::SkippedLocalAhead;
                }
            }
        }
    }

    // Optionally refresh stale pleme-io git pins (branch=main resolved to a
    // GC'd commit) to their live HEADs BEFORE building — otherwise gen
    // build's prefetch hangs on the dead rev. This intentionally changes
    // Cargo.lock; we stage it with the commit below.
    if opts.refresh_git_deps {
        if let Err(detail) = refresh_git_deps(repo, opts.build_timeout_secs) {
            return MigrateOutcome::Failed {
                category: MigrateFailure::BuildFailed,
                detail: format!("refresh git deps: {detail}"),
                elapsed_ms: ms(),
            };
        }
    }

    // Build in a KILLABLE subprocess with a timeout. A stale git pin that
    // hangs a fetch would otherwise hang the whole run; here the build's
    // process group is killed and the repo is skipped. gen build is
    // read-only on Cargo.lock — we verify that below (post-refresh).
    let lock_before = std::fs::read(repo.join("Cargo.lock")).ok();
    match run_build_with_timeout(repo, &opts.gen_bin, opts.build_timeout_secs) {
        BuildResult::Ok => {}
        BuildResult::TimedOut => {
            // A timed-out build may have written a partial spec; restore.
            if build_spec_tracked(repo) {
                let _ = run_git(repo, &["checkout", "--", BUILD_SPEC]);
            }
            let _ = remove_if_untracked(repo, DELTA);
            return MigrateOutcome::SkippedBuildTimeout;
        }
        BuildResult::Failed(detail) => {
            return MigrateOutcome::Failed {
                category: MigrateFailure::BuildFailed,
                detail,
                elapsed_ms: ms(),
            };
        }
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

    // ★ CORE DELTA-ONLY INVARIANT: a delta-only repo MUST commit Cargo.lock
    // — substrate's reconstruct reads it (delta carries only the resolver
    // edge-delta, not the full lock). Detect + self-resolve the gitignored-
    // lock case (a committed delta without a committed lock is orphaned →
    // reconstruct fails → falls back to IFD). Un-ignore the lock so it can
    // be tracked; it is staged unconditionally below.
    if let Err(e) = ensure_lock_committable(repo) {
        return MigrateOutcome::Failed {
            category: MigrateFailure::GitStageFailed,
            detail: e,
            elapsed_ms: ms(),
        };
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
    // Retire the deprecated crate2nix-era crate-hashes.json sidecar too,
    // if a repo still tracks it (untrack + gitignore). Its deletion is
    // staged below.
    let crate_hashes_retired =
        run_git(repo, &["ls-files", "--error-unmatch", CRATE_HASHES]).is_ok();
    if crate_hashes_retired {
        let _ = run_git(repo, &["rm", "--quiet", "--cached", "--", CRATE_HASHES]);
        let _ = ensure_gitignored(repo, CRATE_HASHES);
    }

    // Stage the delta-only artifacts (path-targeted — never `-A`): the
    // delta, the .gitignore (build-spec retired + lock un-ignored), and
    // Cargo.lock (the reconstruction base — committing it is the core
    // invariant). Plus the crate-hashes.json removal when retired.
    let mut stage: Vec<&str> = vec!["add", "--", DELTA, ".gitignore"];
    if repo.join("Cargo.lock").exists() {
        stage.push("Cargo.lock");
    }
    if crate_hashes_retired {
        stage.push(CRATE_HASHES);
    }
    if let Err(e) = run_git(repo, &stage) {
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
        if !ALLOWED.contains(&path) && !is_discardable_artifact(path) {
            return Ok(Some(format!("{status} {path}")));
        }
    }
    Ok(None)
}

/// Discard stale build-artifact dirt (`target/`, `crate-hashes.json`) so a
/// repo whose only dirt is those can be migrated. Safe: callers invoke
/// this only after `blocking_dirty` confirms no real source/lock WIP, so
/// the discard touches nothing the operator cares about.
fn discard_stale_artifacts(repo: &Path) {
    let _ = run_git(repo, &["checkout", "--", "target", CRATE_HASHES]);
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

/// Make Cargo.lock committable: drop any `Cargo.lock` / `/Cargo.lock`
/// ignore line from `<repo>/.gitignore` (a no-op if absent). The
/// delta-only invariant requires a committed lock; a gitignored lock
/// would orphan the delta. Idempotent.
fn ensure_lock_committable(repo: &Path) -> Result<(), String> {
    let gi = repo.join(".gitignore");
    let cur = match std::fs::read_to_string(&gi) {
        Ok(s) => s,
        Err(_) => return Ok(()), // no .gitignore → lock isn't ignored
    };
    let kept: Vec<&str> = cur
        .lines()
        .filter(|l| {
            let t = l.trim();
            t != "Cargo.lock" && t != "/Cargo.lock"
        })
        .collect();
    if kept.len() == cur.lines().count() {
        return Ok(()); // nothing ignored the lock
    }
    let mut new = kept.join("\n");
    if !new.is_empty() {
        new.push('\n');
    }
    std::fs::write(&gi, new).map_err(|e| format!("rewrite .gitignore: {e}"))
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

/// Outcome of a timed build subprocess.
enum BuildResult {
    Ok,
    TimedOut,
    Failed(String),
}

/// Run `<gen_bin> build <repo>` (multi-target, no commit) as a subprocess
/// with a timeout. The child is its own process-group leader, so on
/// timeout we kill the whole group (`kill -KILL -<pid>`) — taking down any
/// hung `git fetch` it spawned, not just the parent. Unix-only (gen
/// targets darwin/linux).
fn run_build_with_timeout(repo: &Path, gen_bin: &Path, timeout_secs: u64) -> BuildResult {
    let rp = repo.to_string_lossy();
    run_proc_with_timeout(gen_bin, &["build", rp.as_ref()], timeout_secs)
}

/// Spawn `program args` in its own process group with a timeout, stdio
/// nulled. On timeout, kill the whole group (`kill -KILL -<pid>`) so any
/// hung child (e.g. a `git fetch` of a GC'd rev) dies too. Unix-only.
fn run_proc_with_timeout(program: &Path, args: &[&str], timeout_secs: u64) -> BuildResult {
    use std::os::unix::process::CommandExt;
    use std::time::Duration;

    let mut child = match Command::new(program)
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .process_group(0) // new group: pgid == child pid
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return BuildResult::Failed(format!("spawn {}: {e}", program.display())),
    };
    let pid = child.id();
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return if status.success() {
                    BuildResult::Ok
                } else {
                    BuildResult::Failed(format!(
                        "{} exited {}",
                        program.display(),
                        status.code().unwrap_or(-1)
                    ))
                };
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = Command::new("kill")
                        .arg("-KILL")
                        .arg(format!("-{pid}"))
                        .status();
                    let _ = child.kill();
                    let _ = child.wait();
                    return BuildResult::TimedOut;
                }
                std::thread::sleep(Duration::from_millis(200));
            }
            Err(e) => return BuildResult::Failed(format!("wait {}: {e}", program.display())),
        }
    }
}

/// Refresh stale pleme-io `branch="main"` git pins in the lock to their
/// live HEADs via `cargo update -p <dep>`, fixing GC'd-rev fetch hangs.
/// Returns Ok(true) if the lock changed. Killable + timed (in case a
/// dep's own main is also unreachable).
fn refresh_git_deps(repo: &Path, timeout_secs: u64) -> Result<bool, String> {
    let lock_path = repo.join("Cargo.lock");
    let lock = std::fs::read_to_string(&lock_path).map_err(|e| format!("read lock: {e}"))?;
    // Names of packages whose source is a pleme-io git repo.
    let mut names: Vec<String> = Vec::new();
    let mut cur: Option<String> = None;
    for line in lock.lines() {
        if let Some(rest) = line.strip_prefix("name = \"") {
            cur = rest.strip_suffix('"').map(str::to_string);
        } else if line.starts_with("source = \"git+https://github.com/pleme-io/") {
            if let Some(n) = cur.take() {
                names.push(n);
            }
        }
    }
    names.sort();
    names.dedup();
    if names.is_empty() {
        return Ok(false);
    }
    let before = std::fs::read(&lock_path).ok();
    let manifest = repo.join("Cargo.toml").to_string_lossy().into_owned();
    // Update each git dep individually + TOLERATE per-dep failure: a
    // transitive pleme dep isn't a directly addressable `-p` spec (cargo
    // exits 101), but the directly-pinned dead one will refresh. A timeout
    // is the only hard error (a dep whose own main is unreachable).
    for n in &names {
        match run_proc_with_timeout(
            Path::new("cargo"),
            &["update", "--manifest-path", &manifest, "-p", n],
            timeout_secs,
        ) {
            BuildResult::TimedOut => return Err(format!("cargo update -p {n} timed out")),
            BuildResult::Ok | BuildResult::Failed(_) => {} // tolerate non-addressable specs
        }
    }
    Ok(std::fs::read(&lock_path).ok() != before)
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

    fn test_opts() -> MigrateOpts {
        MigrateOpts {
            push: false,
            bot_identity: false,
            gen_bin: PathBuf::from("/nonexistent-gen"),
            build_timeout_secs: 5,
            jobs: 1,
            refresh_git_deps: false,
        }
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
    fn ensure_lock_committable_drops_lock_ignore() {
        let dir = temp_git_repo("lockgi");
        write(&dir, ".gitignore", "/target\nCargo.build-spec.json\nCargo.lock\n");
        ensure_lock_committable(&dir).unwrap();
        let gi = std::fs::read_to_string(dir.join(".gitignore")).unwrap();
        assert!(!gi.lines().any(|l| l.trim() == "Cargo.lock"), "lock ignore dropped");
        assert!(gi.lines().any(|l| l.trim() == "Cargo.build-spec.json"), "build-spec ignore kept");
        assert!(gi.lines().any(|l| l.trim() == "/target"), "other ignores kept");
        // idempotent + no-op when lock isn't ignored
        ensure_lock_committable(&dir).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join(".gitignore")).unwrap(), gi);
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
    fn blocking_dirty_allows_stale_artifacts_but_blocks_source() {
        let dir = temp_git_repo("artifacts");
        std::fs::create_dir_all(dir.join("target")).unwrap();
        write(&dir, "target/x.o", "a");
        write(&dir, "crate-hashes.json", "{}");
        write(&dir, "src.rs", "a");
        run_git(&dir, &["add", "-A"]).unwrap();
        run_git(&dir, &["commit", "--quiet", "-m", "init"]).unwrap();
        // Dirty ONLY stale artifacts → not blocking.
        write(&dir, "target/x.o", "b");
        std::fs::remove_file(dir.join("crate-hashes.json")).unwrap();
        assert_eq!(blocking_dirty(&dir).unwrap(), None, "stale artifacts must not block");
        // Now also touch a source file → blocking.
        write(&dir, "src.rs", "b");
        assert!(
            blocking_dirty(&dir).unwrap().is_some(),
            "real source WIP must block"
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
            migrate_one(&dir, &test_opts()),
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
            migrate_one(&dir, &test_opts()),
            MigrateOutcome::SkippedNotAGitRepo
        ));
    }

    #[test]
    fn build_timeout_kills_hung_process_group() {
        use std::os::unix::fs::PermissionsExt;
        use std::time::Duration;
        let dir = temp_git_repo("timeout");
        // A fake `gen` that ignores its args and hangs well past the timeout.
        let fakegen = dir.join("fakegen.sh");
        std::fs::write(&fakegen, "#!/bin/sh\nsleep 30\n").unwrap();
        std::fs::set_permissions(&fakegen, std::fs::Permissions::from_mode(0o755)).unwrap();
        let start = Instant::now();
        let r = run_build_with_timeout(&dir, &fakegen, 1);
        assert!(matches!(r, BuildResult::TimedOut), "expected TimedOut, got {r:?}");
        assert!(
            start.elapsed() < Duration::from_secs(8),
            "timeout must kill ~1s, not wait the full 30s sleep"
        );
    }
}

#[cfg(test)]
impl std::fmt::Debug for BuildResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BuildResult::Ok => write!(f, "Ok"),
            BuildResult::TimedOut => write!(f, "TimedOut"),
            BuildResult::Failed(d) => write!(f, "Failed({d})"),
        }
    }
}
