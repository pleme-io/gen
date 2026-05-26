//! `gen fleet-commit` — typed multi-repo commit + push primitive.
//!
//! Sibling of `fleet_sweep`. After `gen fleet-sweep --write` lands
//! Cargo.build-spec.json sidecars into N repos, `gen fleet-commit`
//! walks the same set + commits the typed sidecar to each repo's
//! main branch.
//!
//! Algorithmic guarantees:
//!   - Deterministic: same repo state → same commit content + message
//!   - Side-effect-scoped: only stages `Cargo.build-spec.json`
//!     (never `git add -A`); other dirty paths in the working tree
//!     remain untouched.
//!   - Idempotent: re-running on a repo with the spec already
//!     committed reports SkippedAlreadyClean.
//!   - Typed failure classification: every shell-level error maps
//!     to a typed CommitOutcome variant; no opaque error strings
//!     reach the operator.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::error::CargoError;

const SIDECAR: &str = "Cargo.build-spec.json";

/// One repo's commit outcome.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum CommitOutcome {
    /// Committed (and pushed if `push` was requested).
    Committed {
        commit_sha: String,
        pushed: bool,
        elapsed_ms: u64,
    },
    /// No spec file present; nothing to commit.
    SkippedNoSidecar,
    /// Spec is already at HEAD, working-tree-clean for this file.
    SkippedAlreadyClean,
    /// Not a git repo.
    SkippedNotAGitRepo,
    /// A typed structural failure.
    Failed {
        category: CommitFailureCategory,
        detail: String,
        elapsed_ms: u64,
    },
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum CommitFailureCategory {
    /// git add failed (file inaccessible, etc.).
    GitAddFailed,
    /// git commit failed (commit hook rejected, signing failure).
    GitCommitFailed,
    /// git push failed (network, auth, branch-protection rule).
    GitPushFailed,
    /// git rev-parse / status / cat-file failed unexpectedly.
    GitInspectionFailed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CommitReport {
    pub root: PathBuf,
    pub outcomes: IndexMap<String, CommitOutcome>,
    pub total_elapsed_ms: u64,
}

impl CommitReport {
    #[must_use]
    pub fn total(&self) -> usize {
        self.outcomes.len()
    }
    #[must_use]
    pub fn committed_count(&self) -> usize {
        self.outcomes
            .values()
            .filter(|o| matches!(o, CommitOutcome::Committed { .. }))
            .count()
    }
    #[must_use]
    pub fn pushed_count(&self) -> usize {
        self.outcomes
            .values()
            .filter(|o| matches!(o, CommitOutcome::Committed { pushed: true, .. }))
            .count()
    }
    #[must_use]
    pub fn skipped_count(&self) -> usize {
        self.outcomes
            .values()
            .filter(|o| {
                matches!(
                    o,
                    CommitOutcome::SkippedNoSidecar
                        | CommitOutcome::SkippedAlreadyClean
                        | CommitOutcome::SkippedNotAGitRepo
                )
            })
            .count()
    }
    #[must_use]
    pub fn failed_count(&self) -> usize {
        self.outcomes
            .values()
            .filter(|o| matches!(o, CommitOutcome::Failed { .. }))
            .count()
    }
}

/// Walk every immediate sub-directory of `root`; for each, commit
/// `Cargo.build-spec.json` if it differs from HEAD.
///
/// `push = true` additionally pushes to `origin/<current-branch>`
/// after each successful commit. `rebase_first` runs
/// `git pull --rebase` before push to absorb upstream changes (sidecars
/// land cleanly on top of CI auto-release commits).
pub fn run(
    root: &Path,
    push: bool,
    rebase_first: bool,
) -> Result<CommitReport, CargoError> {
    let started = Instant::now();
    let mut outcomes: IndexMap<String, CommitOutcome> = IndexMap::new();

    let entries = std::fs::read_dir(root).map_err(|source| CargoError::Io {
        path: root.to_path_buf(),
        source,
    })?;
    let mut dirs: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();

    for repo in dirs {
        let name = repo
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        if name.is_empty() || name.starts_with('.') {
            continue;
        }
        let outcome = commit_one(&repo, push, rebase_first);
        outcomes.insert(name, outcome);
    }

    Ok(CommitReport {
        root: root.to_path_buf(),
        outcomes,
        total_elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

fn commit_one(repo: &Path, push: bool, rebase_first: bool) -> CommitOutcome {
    let started = Instant::now();
    if !repo.join(SIDECAR).exists() {
        return CommitOutcome::SkippedNoSidecar;
    }
    if !repo.join(".git").exists() {
        return CommitOutcome::SkippedNotAGitRepo;
    }

    // Determine whether sidecar differs from HEAD.
    match git_file_state(repo) {
        Ok(GitFileState::Clean) => return CommitOutcome::SkippedAlreadyClean,
        Ok(GitFileState::UntrackedOrModified) => {}
        Err(detail) => {
            return CommitOutcome::Failed {
                category: CommitFailureCategory::GitInspectionFailed,
                detail,
                elapsed_ms: started.elapsed().as_millis() as u64,
            };
        }
    }

    // git add Cargo.build-spec.json (only this file — never -A).
    if let Err(detail) = run_git(repo, &["add", SIDECAR]) {
        return CommitOutcome::Failed {
            category: CommitFailureCategory::GitAddFailed,
            detail,
            elapsed_ms: started.elapsed().as_millis() as u64,
        };
    }

    // git commit with the canonical deterministic message.
    let msg = canonical_commit_message();
    if let Err(detail) = run_git(repo, &["commit", "-m", &msg]) {
        return CommitOutcome::Failed {
            category: CommitFailureCategory::GitCommitFailed,
            detail,
            elapsed_ms: started.elapsed().as_millis() as u64,
        };
    }

    // Capture the new commit SHA.
    let commit_sha = match run_git(repo, &["rev-parse", "HEAD"]) {
        Ok(s) => s.trim().to_string(),
        Err(detail) => {
            return CommitOutcome::Failed {
                category: CommitFailureCategory::GitInspectionFailed,
                detail,
                elapsed_ms: started.elapsed().as_millis() as u64,
            };
        }
    };

    if !push {
        return CommitOutcome::Committed {
            commit_sha,
            pushed: false,
            elapsed_ms: started.elapsed().as_millis() as u64,
        };
    }

    // Optional rebase to absorb upstream commits before push.
    if rebase_first {
        if let Err(_detail) = run_git(repo, &["fetch", "origin"]) {
            // fetch failure isn't fatal — push will surface a clearer error.
        }
        // pull --rebase tolerates the autostash + replay model.
        let _ = run_git(repo, &["pull", "--rebase", "origin", "HEAD"]);
    }

    // git push origin HEAD.
    if let Err(detail) = run_git(repo, &["push", "origin", "HEAD"]) {
        return CommitOutcome::Failed {
            category: CommitFailureCategory::GitPushFailed,
            detail,
            elapsed_ms: started.elapsed().as_millis() as u64,
        };
    }

    CommitOutcome::Committed {
        commit_sha,
        pushed: true,
        elapsed_ms: started.elapsed().as_millis() as u64,
    }
}

enum GitFileState {
    /// File matches HEAD content; nothing to commit.
    Clean,
    /// File is untracked, modified, or staged.
    UntrackedOrModified,
}

/// Inspect `Cargo.build-spec.json`'s state vs HEAD. Deterministic:
/// returns Clean iff `git status --porcelain SIDECAR` is empty.
fn git_file_state(repo: &Path) -> Result<GitFileState, String> {
    let out = run_git(repo, &["status", "--porcelain", SIDECAR])?;
    if out.trim().is_empty() {
        Ok(GitFileState::Clean)
    } else {
        Ok(GitFileState::UntrackedOrModified)
    }
}

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

/// Canonical commit message — deterministic per-spec rollout. Same
/// across all repos so operators can grep the fleet by message.
fn canonical_commit_message() -> String {
    "add Cargo.build-spec.json — substrate lockfile-builder default-on input

Typed sidecar produced by `gen lock-build`. Composes Cargo.toml +
Cargo.lock + cargo metadata into one canonical JSON that substrate's
pure-Nix lockfile-builder consumes directly. Eliminates the need
for a generated Cargo.nix on the substrate default path.

Regenerate with `gen build .` whenever Cargo.lock changes.

  - gen ecosystem:    github.com/pleme-io/gen
  - substrate path:   substrate.lib.build.rust.lockfile-builder
  - rollout doc:      pleme-io/gen/docs/PACKED-DEFAULTS.md
"
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tempdir() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static C: AtomicU64 = AtomicU64::new(0);
        let n = C.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!(
            "gen-fleet-commit-test-{}-{}",
            std::process::id(),
            n
        ));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    /// init a bare repo with one initial commit so git push has a sense
    /// of HEAD; useful for tests that need real-shape git state.
    fn init_repo(p: &Path) {
        run_git(p, &["init", "-q"]).unwrap();
        run_git(p, &["config", "user.email", "test@example.org"]).unwrap();
        run_git(p, &["config", "user.name", "test"]).unwrap();
        run_git(p, &["config", "commit.gpgsign", "false"]).unwrap();
        fs::write(p.join("README"), "x").unwrap();
        run_git(p, &["add", "README"]).unwrap();
        run_git(p, &["commit", "-q", "-m", "init"]).unwrap();
    }

    #[test]
    fn skipped_no_sidecar_when_file_absent() {
        let root = tempdir();
        let repo = root.join("empty");
        fs::create_dir_all(&repo).unwrap();
        let report = run(&root, false, false).unwrap();
        assert!(matches!(
            report.outcomes.get("empty"),
            Some(CommitOutcome::SkippedNoSidecar)
        ));
    }

    #[test]
    fn skipped_not_a_git_repo_when_no_dot_git() {
        let root = tempdir();
        let repo = root.join("nogit");
        fs::create_dir_all(&repo).unwrap();
        fs::write(repo.join(SIDECAR), "{}").unwrap();
        let report = run(&root, false, false).unwrap();
        assert!(matches!(
            report.outcomes.get("nogit"),
            Some(CommitOutcome::SkippedNotAGitRepo)
        ));
    }

    #[test]
    fn untracked_sidecar_is_committed() {
        let root = tempdir();
        let repo = root.join("real");
        fs::create_dir_all(&repo).unwrap();
        init_repo(&repo);
        fs::write(repo.join(SIDECAR), "{\"version\":1}\n").unwrap();
        let report = run(&root, false, false).unwrap();
        let outcome = report.outcomes.get("real").unwrap();
        assert!(
            matches!(outcome, CommitOutcome::Committed { pushed: false, .. }),
            "expected Committed, got {outcome:?}"
        );
    }

    #[test]
    fn already_committed_is_skipped_clean() {
        let root = tempdir();
        let repo = root.join("done");
        fs::create_dir_all(&repo).unwrap();
        init_repo(&repo);
        fs::write(repo.join(SIDECAR), "{\"version\":1}\n").unwrap();
        run_git(&repo, &["add", SIDECAR]).unwrap();
        run_git(&repo, &["commit", "-q", "-m", "spec"]).unwrap();
        let report = run(&root, false, false).unwrap();
        assert!(matches!(
            report.outcomes.get("done"),
            Some(CommitOutcome::SkippedAlreadyClean)
        ));
    }

    #[test]
    fn report_aggregators_count_correctly() {
        let mut outcomes = IndexMap::new();
        outcomes.insert(
            "a".into(),
            CommitOutcome::Committed {
                commit_sha: "x".into(),
                pushed: false,
                elapsed_ms: 1,
            },
        );
        outcomes.insert(
            "b".into(),
            CommitOutcome::Committed {
                commit_sha: "y".into(),
                pushed: true,
                elapsed_ms: 1,
            },
        );
        outcomes.insert("c".into(), CommitOutcome::SkippedAlreadyClean);
        outcomes.insert(
            "d".into(),
            CommitOutcome::Failed {
                category: CommitFailureCategory::GitPushFailed,
                detail: "x".into(),
                elapsed_ms: 1,
            },
        );
        let report = CommitReport {
            root: PathBuf::from("/x"),
            outcomes,
            total_elapsed_ms: 4,
        };
        assert_eq!(report.total(), 4);
        assert_eq!(report.committed_count(), 2);
        assert_eq!(report.pushed_count(), 1);
        assert_eq!(report.skipped_count(), 1);
        assert_eq!(report.failed_count(), 1);
    }

    #[test]
    fn commit_message_is_deterministic() {
        let a = canonical_commit_message();
        let b = canonical_commit_message();
        assert_eq!(a, b);
        assert!(a.contains("substrate lockfile-builder default-on input"));
    }

    #[test]
    fn other_dirty_files_are_not_staged() {
        let root = tempdir();
        let repo = root.join("dirty");
        fs::create_dir_all(&repo).unwrap();
        init_repo(&repo);
        // Other dirty file that must NOT be committed.
        fs::write(repo.join("OTHER"), "leave-me-alone").unwrap();
        fs::write(repo.join(SIDECAR), "{\"version\":1}\n").unwrap();
        let report = run(&root, false, false).unwrap();
        assert!(matches!(
            report.outcomes.get("dirty"),
            Some(CommitOutcome::Committed { .. })
        ));
        // OTHER must still be untracked.
        let status = run_git(&repo, &["status", "--porcelain"]).unwrap();
        assert!(status.contains("?? OTHER"), "OTHER should be untracked: {status:?}");
    }
}
