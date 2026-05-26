//! `gen fleet-sweep` — algorithmic fleet rollout primitive.
//!
//! Sibling of `gen build` for the multi-repo case. Discovers every
//! Cargo workspace under a root directory and runs the BuildSpec
//! generator against each, capturing per-repo outcomes in one typed
//! report. Replaces ad-hoc bash loops that previously did fleet
//! rollout work.
//!
//! Per the prime directive: no shell, Rust + typed primitives all
//! the way down. The sweep IS a typed operation, not a script.

use std::path::{Path, PathBuf};
use std::time::Instant;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::build_spec;
use crate::error::CargoError;

/// One repo's sweep outcome — typed, JSON-serializable.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum SweepOutcome {
    /// Spec was generated successfully.
    Ok {
        spec_bytes: usize,
        elapsed_ms: u64,
    },
    /// Repo skipped (no Cargo.toml or no Cargo.lock).
    Skipped { reason: SkipReason },
    /// gen-cargo or cargo metadata failed.
    Failed {
        category: FailureCategory,
        detail: String,
        elapsed_ms: u64,
    },
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SkipReason {
    NoCargoToml,
    NoCargoLock,
}

/// Structural failure categories. Each category corresponds to one
/// algorithmic class of upstream cargo state problem — no one-off
/// corner cases. New cargo-state failure shapes become new variants
/// here, never inline checks in the sweep loop.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FailureCategory {
    /// `cargo metadata` couldn't fetch a git dependency (auth /
    /// network / repo doesn't exist).
    GitFetchFailed,
    /// `cargo metadata` couldn't resolve a version requirement.
    VersionResolutionFailed,
    /// A workspace member's Cargo.toml is missing or invalid.
    WorkspaceMemberInvalid,
    /// gen-cargo's own parse phase failed (Cargo.toml malformed).
    GenParseFailed,
    /// Anything cargo metadata returned that we don't have a
    /// dedicated category for. Reading the detail tells us if we
    /// need a new category.
    OtherCargoError,
}

impl FailureCategory {
    /// Classify a cargo-metadata error string into a structural
    /// category. The classification is pattern-based + algorithmic
    /// (one match arm per category), not corner-case-dispatched.
    fn classify(detail: &str) -> Self {
        if detail.contains("Updating git repository") {
            Self::GitFetchFailed
        } else if detail.contains("failed to select a version") {
            Self::VersionResolutionFailed
        } else if detail.contains("failed to load manifest for workspace member") {
            Self::WorkspaceMemberInvalid
        } else if !detail.contains("cargo metadata") {
            // gen-cargo's own parse fired before cargo did.
            Self::GenParseFailed
        } else {
            Self::OtherCargoError
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SweepReport {
    pub root: PathBuf,
    pub outcomes: IndexMap<String, SweepOutcome>,
    pub total_elapsed_ms: u64,
}

impl SweepReport {
    #[must_use]
    pub fn total(&self) -> usize {
        self.outcomes.len()
    }
    #[must_use]
    pub fn ok_count(&self) -> usize {
        self.outcomes
            .values()
            .filter(|o| matches!(o, SweepOutcome::Ok { .. }))
            .count()
    }
    #[must_use]
    pub fn failed_count(&self) -> usize {
        self.outcomes
            .values()
            .filter(|o| matches!(o, SweepOutcome::Failed { .. }))
            .count()
    }
    #[must_use]
    pub fn skipped_count(&self) -> usize {
        self.outcomes
            .values()
            .filter(|o| matches!(o, SweepOutcome::Skipped { .. }))
            .count()
    }
    #[must_use]
    pub fn total_spec_bytes(&self) -> usize {
        self.outcomes
            .values()
            .filter_map(|o| match o {
                SweepOutcome::Ok { spec_bytes, .. } => Some(*spec_bytes),
                _ => None,
            })
            .sum()
    }

    /// Group failures by structural category.
    #[must_use]
    pub fn failures_by_category(&self) -> IndexMap<FailureCategory, Vec<String>> {
        let mut out: IndexMap<FailureCategory, Vec<String>> = IndexMap::new();
        for (repo, outcome) in &self.outcomes {
            if let SweepOutcome::Failed { category, .. } = outcome {
                out.entry(*category).or_default().push(repo.clone());
            }
        }
        out
    }
}

/// Run a fleet sweep over every immediate sub-directory of `root`.
///
/// `write` controls whether the BuildSpec is persisted to disk:
///   - `false` (default): dry-run, generate-and-discard mode for
///     fleet-health visibility.
///   - `true`: write `Cargo.build-spec.json` into each successful repo
///     so operators can commit them.
pub fn run(root: &Path, write: bool) -> Result<SweepReport, CargoError> {
    let started = Instant::now();
    let mut outcomes: IndexMap<String, SweepOutcome> = IndexMap::new();

    let dirs = std::fs::read_dir(root).map_err(|source| CargoError::Io {
        path: root.to_path_buf(),
        source,
    })?;
    let mut entries: Vec<PathBuf> = dirs
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir())
        .collect();
    entries.sort();

    for entry in entries {
        let repo_name = entry
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        if repo_name.is_empty() || repo_name.starts_with('.') {
            continue;
        }
        let outcome = sweep_one(&entry, write);
        outcomes.insert(repo_name, outcome);
    }

    Ok(SweepReport {
        root: root.to_path_buf(),
        outcomes,
        total_elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

fn sweep_one(repo: &Path, write: bool) -> SweepOutcome {
    if !repo.join("Cargo.toml").exists() {
        return SweepOutcome::Skipped {
            reason: SkipReason::NoCargoToml,
        };
    }
    if !repo.join("Cargo.lock").exists() {
        return SweepOutcome::Skipped {
            reason: SkipReason::NoCargoLock,
        };
    }
    let started = Instant::now();
    let spec_result = if write {
        build_spec::generate_and_write(repo).map(|path| {
            std::fs::metadata(&path).map(|m| m.len() as usize).unwrap_or(0)
        })
    } else {
        build_spec::generate(repo).map(|spec| {
            serde_json::to_string(&spec).map(|s| s.len()).unwrap_or(0)
        })
    };
    let elapsed_ms = started.elapsed().as_millis() as u64;
    match spec_result {
        Ok(spec_bytes) => SweepOutcome::Ok { spec_bytes, elapsed_ms },
        Err(e) => {
            let detail = e.to_string();
            let category = FailureCategory::classify(&detail);
            SweepOutcome::Failed {
                category,
                detail,
                elapsed_ms,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tempdir() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static C: AtomicU64 = AtomicU64::new(0);
        let n = C.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("gen-fleet-sweep-test-{}-{}", std::process::id(), n));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn classifier_recognizes_git_fetch_failure() {
        let detail = "cargo metadata exited with an error: Updating git repository ssh://...";
        assert!(matches!(
            FailureCategory::classify(detail),
            FailureCategory::GitFetchFailed
        ));
    }

    #[test]
    fn classifier_recognizes_version_resolution_failure() {
        let detail = "cargo metadata exited with an error: failed to select a version for ...";
        assert!(matches!(
            FailureCategory::classify(detail),
            FailureCategory::VersionResolutionFailed
        ));
    }

    #[test]
    fn classifier_recognizes_workspace_member_failure() {
        let detail = "cargo metadata exited with an error: failed to load manifest for workspace member ...";
        assert!(matches!(
            FailureCategory::classify(detail),
            FailureCategory::WorkspaceMemberInvalid
        ));
    }

    #[test]
    fn skipped_repos_are_classified() {
        let root = tempdir();
        let empty_repo = root.join("empty");
        fs::create_dir_all(&empty_repo).unwrap();
        let report = run(&root, false).unwrap();
        assert!(matches!(
            report.outcomes.get("empty"),
            Some(SweepOutcome::Skipped {
                reason: SkipReason::NoCargoToml
            })
        ));
    }

    #[test]
    fn cargo_no_lockfile_is_skipped() {
        let root = tempdir();
        let repo = root.join("no-lock");
        fs::create_dir_all(&repo).unwrap();
        fs::write(
            repo.join("Cargo.toml"),
            r#"[package]
name = "x"
version = "0.1.0"
edition = "2024"
"#,
        )
        .unwrap();
        let report = run(&root, false).unwrap();
        assert!(matches!(
            report.outcomes.get("no-lock"),
            Some(SweepOutcome::Skipped {
                reason: SkipReason::NoCargoLock
            })
        ));
    }

    #[test]
    fn report_aggregators_count_correctly() {
        let mut outcomes = IndexMap::new();
        outcomes.insert(
            "a".into(),
            SweepOutcome::Ok {
                spec_bytes: 100,
                elapsed_ms: 5,
            },
        );
        outcomes.insert(
            "b".into(),
            SweepOutcome::Ok {
                spec_bytes: 200,
                elapsed_ms: 5,
            },
        );
        outcomes.insert(
            "c".into(),
            SweepOutcome::Failed {
                category: FailureCategory::GitFetchFailed,
                detail: "x".into(),
                elapsed_ms: 5,
            },
        );
        outcomes.insert(
            "d".into(),
            SweepOutcome::Skipped {
                reason: SkipReason::NoCargoToml,
            },
        );
        let report = SweepReport {
            root: PathBuf::from("/x"),
            outcomes,
            total_elapsed_ms: 20,
        };
        assert_eq!(report.total(), 4);
        assert_eq!(report.ok_count(), 2);
        assert_eq!(report.failed_count(), 1);
        assert_eq!(report.skipped_count(), 1);
        assert_eq!(report.total_spec_bytes(), 300);
        let by_cat = report.failures_by_category();
        assert_eq!(by_cat.get(&FailureCategory::GitFetchFailed).map(Vec::len), Some(1));
    }
}

// FailureCategory needs Hash + Eq for failures_by_category's IndexMap key.
impl std::hash::Hash for FailureCategory {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
    }
}

impl std::cmp::PartialEq for FailureCategory {
    fn eq(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}

impl std::cmp::Eq for FailureCategory {}
