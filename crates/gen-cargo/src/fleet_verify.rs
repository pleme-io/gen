//! `gen fleet-verify` — typed read-only verification that a fleet of
//! repos reached the DELTA-ONLY end state.
//!
//! The typed companion to `fleet_migrate`: instead of a fragile shell
//! loop, each repo's REMOTE (`origin/<branch>`) state is fetched and
//! classified into one `VerifyState`. Used to confirm a conversion
//! landed and to find stragglers (still-build-spec, still-IFD-only).
//!
//! Verification is by the committed git state on the remote branch:
//!   - `Cargo.gen.lock` tracked + `Cargo.build-spec.json` absent  → DeltaOnly
//!   - `Cargo.build-spec.json` tracked                            → StillBuildSpec
//!   - build-spec gitignored, no delta                            → IfdOnlyNoDelta
//!   - none of the above                                          → NotParticipating

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::error::CargoError;

const BUILD_SPEC: &str = "Cargo.build-spec.json";
const DELTA: &str = "Cargo.gen.lock";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum VerifyState {
    /// The goal: delta + Cargo.lock committed, build-spec retired.
    DeltaOnly,
    /// Delta committed but NO committed Cargo.lock — reconstruct can't
    /// read the lock on a fresh checkout, so the delta is orphaned and
    /// the repo silently falls back to IFD. Not truly delta-only.
    OrphanedDeltaNoLock,
    /// Delta + lock committed, but MISSING the gen-spec self-regen CI
    /// (.github/workflows/gen-spec.yml). The delta is correct now but will
    /// go stale on the next lock bump (auto-release / dep update) with
    /// nothing to regenerate it. Not self-sustaining.
    DeltaOnlyNoCi,
    /// Build-spec still tracked (not yet retired).
    StillBuildSpec,
    /// Build-spec retired + gitignored, but no delta (legacy IFD-only).
    IfdOnlyNoDelta,
    /// Neither build-spec nor delta nor the retire marker — not a
    /// participating repo (or branch missing).
    NotParticipating,
    /// Inspection error (not a git repo, no remote branch, etc.).
    Error { detail: String },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VerifyReport {
    pub rows: IndexMap<String, VerifyState>,
    pub total_elapsed_ms: u64,
}

impl VerifyReport {
    pub fn count(&self, want: &VerifyState) -> usize {
        self.rows.values().filter(|s| *s == want).count()
    }
    pub fn delta_only(&self) -> usize {
        self.count(&VerifyState::DeltaOnly)
    }
    pub fn all_delta_only(&self) -> bool {
        !self.rows.is_empty() && self.rows.values().all(|s| *s == VerifyState::DeltaOnly)
    }
}

/// Options.
#[derive(Clone, Copy, Debug)]
pub struct VerifyOpts {
    /// `git fetch origin <branch>` before reading (authoritative remote
    /// state). When false, reads the local remote-tracking ref as-is.
    pub fetch: bool,
    /// Concurrent repos.
    pub jobs: usize,
}

/// Verify every repo, up to `opts.jobs` concurrently. Order preserved.
pub fn verify(repos: &[PathBuf], opts: VerifyOpts) -> Result<VerifyReport, CargoError> {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let started = Instant::now();
    let jobs = opts.jobs.max(1);
    let next = AtomicUsize::new(0);
    let collected: Mutex<Vec<(usize, String, VerifyState)>> = Mutex::new(Vec::new());

    std::thread::scope(|scope| {
        for _ in 0..jobs {
            scope.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::SeqCst);
                    if i >= repos.len() {
                        break;
                    }
                    let repo = &repos[i];
                    let name = repo
                        .file_name()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_else(|| repo.display().to_string());
                    let state = verify_one(repo, opts.fetch);
                    collected.lock().unwrap().push((i, name, state));
                }
            });
        }
    });

    let mut rows = collected.into_inner().unwrap();
    rows.sort_by_key(|(i, _, _)| *i);
    Ok(VerifyReport {
        rows: rows.into_iter().map(|(_, n, s)| (n, s)).collect(),
        total_elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

/// Classify one repo's remote-branch state.
pub fn verify_one(repo: &Path, fetch: bool) -> VerifyState {
    if !repo.join(".git").exists() {
        return VerifyState::Error {
            detail: "not a git repo".into(),
        };
    }
    let branch = match git(repo, &["rev-parse", "--abbrev-ref", "HEAD"]) {
        Ok(b) => b.trim().to_string(),
        Err(e) => return VerifyState::Error { detail: e },
    };
    if fetch {
        let _ = git(repo, &["fetch", "--quiet", "origin", &branch]);
    }
    let remote = format!("origin/{branch}");
    let tree = |rel: &str| {
        git(repo, &["ls-tree", &remote, rel])
            .map(|o| !o.trim().is_empty())
            .unwrap_or(false)
    };
    let bs_tracked = tree(BUILD_SPEC);
    let delta_tracked = tree(DELTA);
    let lock_tracked = tree("Cargo.lock");
    let bs_ignored = git(repo, &["show", &format!("{remote}:.gitignore")])
        .map(|gi| gi.lines().any(|l| l.trim() == BUILD_SPEC))
        .unwrap_or(false);

    let ci_present = tree(".github/workflows/gen-spec.yml");
    if delta_tracked && !bs_tracked {
        // True, self-sustaining delta-only = committed Cargo.lock to
        // reconstruct from + the gen-spec CI to keep the delta fresh.
        if !lock_tracked {
            VerifyState::OrphanedDeltaNoLock
        } else if !ci_present {
            VerifyState::DeltaOnlyNoCi
        } else {
            VerifyState::DeltaOnly
        }
    } else if bs_tracked {
        VerifyState::StillBuildSpec
    } else if bs_ignored {
        VerifyState::IfdOnlyNoDelta
    } else {
        VerifyState::NotParticipating
    }
}

fn git(repo: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .map_err(|e| format!("git {args:?}: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git {args:?} → {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("genfv-{}-{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        git(&d, &["init", "--quiet"]).unwrap();
        git(&d, &["config", "user.email", "t@t"]).unwrap();
        git(&d, &["config", "user.name", "t"]).unwrap();
        // a local "origin/<branch>" so verify_one can read it without a remote
        d
    }

    fn commit_all(d: &Path) {
        git(d, &["add", "-A"]).unwrap();
        git(d, &["commit", "--quiet", "-m", "x"]).unwrap();
    }

    // Point origin/<branch> at HEAD via a local alias so verify_one (no
    // network) classifies the committed tree.
    fn alias_remote(d: &Path) {
        let branch = git(d, &["rev-parse", "--abbrev-ref", "HEAD"])
            .unwrap()
            .trim()
            .to_string();
        git(
            d,
            &[
                "update-ref",
                &format!("refs/remotes/origin/{branch}"),
                "HEAD",
            ],
        )
        .unwrap();
    }

    fn write_gen_spec_ci(d: &Path) {
        std::fs::create_dir_all(d.join(".github/workflows")).unwrap();
        std::fs::write(d.join(".github/workflows/gen-spec.yml"), "name: gen-spec\n").unwrap();
    }

    #[test]
    fn classifies_delta_only() {
        let d = repo("delta");
        std::fs::write(d.join(DELTA), "x").unwrap();
        std::fs::write(d.join("Cargo.lock"), "x").unwrap();
        std::fs::write(d.join(".gitignore"), "Cargo.build-spec.json\n").unwrap();
        write_gen_spec_ci(&d); // self-regen CI present → truly delta-only
        commit_all(&d);
        alias_remote(&d);
        assert_eq!(verify_one(&d, false), VerifyState::DeltaOnly);
    }

    #[test]
    fn classifies_delta_only_no_ci() {
        let d = repo("delta-noci");
        std::fs::write(d.join(DELTA), "x").unwrap();
        std::fs::write(d.join("Cargo.lock"), "x").unwrap();
        std::fs::write(d.join(".gitignore"), "Cargo.build-spec.json\n").unwrap();
        // no gen-spec.yml → delta correct but not self-sustaining
        commit_all(&d);
        alias_remote(&d);
        assert_eq!(verify_one(&d, false), VerifyState::DeltaOnlyNoCi);
    }

    #[test]
    fn classifies_orphaned_delta_no_lock() {
        // delta committed but Cargo.lock gitignored/uncommitted → orphaned.
        let d = repo("orphan");
        std::fs::write(d.join(DELTA), "x").unwrap();
        std::fs::write(d.join(".gitignore"), "Cargo.build-spec.json\nCargo.lock\n").unwrap();
        commit_all(&d);
        alias_remote(&d);
        assert_eq!(verify_one(&d, false), VerifyState::OrphanedDeltaNoLock);
    }

    #[test]
    fn classifies_still_build_spec() {
        let d = repo("bs");
        std::fs::write(d.join(BUILD_SPEC), "{}").unwrap();
        commit_all(&d);
        alias_remote(&d);
        assert_eq!(verify_one(&d, false), VerifyState::StillBuildSpec);
    }

    #[test]
    fn classifies_ifd_only() {
        let d = repo("ifd");
        std::fs::write(d.join(".gitignore"), "Cargo.build-spec.json\n").unwrap();
        std::fs::write(d.join("README"), "x").unwrap();
        commit_all(&d);
        alias_remote(&d);
        assert_eq!(verify_one(&d, false), VerifyState::IfdOnlyNoDelta);
    }

    #[test]
    fn classifies_not_participating() {
        let d = repo("np");
        std::fs::write(d.join("README"), "x").unwrap();
        commit_all(&d);
        alias_remote(&d);
        assert_eq!(verify_one(&d, false), VerifyState::NotParticipating);
    }
}
