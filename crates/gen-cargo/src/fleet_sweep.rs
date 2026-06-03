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
    pub fn classify(detail: &str) -> Self {
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

/// Per-kind concurrent-job cap for the fleet-sweep Dag. Sized to
/// balance throughput (more jobs = faster sweep) against
/// cargo-metadata's own concurrency cost (each job runs an offline
/// cargo subprocess). 16 matches tend's reconcile default + has
/// been validated on the 118-repo pleme-io workspace.
pub const DEFAULT_BUDGET: u32 = 16;

/// Run a fleet sweep over every immediate sub-directory of `root`.
///
/// Shigoto-native: each repo is one `GenBuildJob` in a `Dag`, run
/// through `InProcessScheduler` with the per-kind budget capping
/// concurrency at [`DEFAULT_BUDGET`]. Outputs flow through an
/// `InMemorySink<SweepOutcome>`; post-tick drain assembles the
/// `SweepReport` with the same shape the sequential predecessor
/// produced.
///
/// `write` controls whether the BuildSpec is persisted to disk:
///   - `false` (default): dry-run, generate-and-discard mode for
///     fleet-health visibility.
///   - `true`: write `Cargo.build-spec.json` into each successful repo
///     so operators can commit them.
pub fn run(root: &Path, write: bool) -> Result<SweepReport, CargoError> {
    // Fleet-sweep context — every per-repo cargo metadata must be
    // hermetic. Sets the build_spec offline posture (passes
    // `--offline` + `GIT_TERMINAL_PROMPT=0`) so a stale registry
    // entry or missing git checkout fails as a typed error rather
    // than prompting the operator for HTTPS credentials mid-sweep.
    // The IFD path inside nix builders does NOT set this var — it
    // needs the default cargo-metadata behaviour because the
    // sandbox handles the registry layout itself.
    //
    // Safety: process-global env var. Sweeps are short-lived; the
    // mutation lives for the duration of the sweep and is what
    // `build_spec::generate_for_target` checks on every invocation.
    unsafe { std::env::set_var("GEN_CARGO_METADATA_OFFLINE", "1") };

    let started = Instant::now();

    let dirs = std::fs::read_dir(root).map_err(|source| CargoError::Io {
        path: root.to_path_buf(),
        source,
    })?;
    let mut entries: Vec<PathBuf> = dirs
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir())
        .collect();
    entries.sort();

    let repos: Vec<(String, PathBuf)> = entries
        .into_iter()
        .filter_map(|entry| {
            let repo_name = entry
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            if repo_name.is_empty() || repo_name.starts_with('.') {
                None
            } else {
                Some((repo_name, entry))
            }
        })
        .collect();

    let outcomes = run_via_scheduler(&repos, write)?;

    Ok(SweepReport {
        root: root.to_path_buf(),
        outcomes,
        total_elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

/// Scheduler-driven fan-out. Builds a Dag of one `GenBuildJob` per
/// repo, registers them all, and ticks until every Job has
/// terminated. Outputs come back via `InMemorySink<SweepOutcome>`;
/// `SweepReport` ordering matches the input order via the
/// `repos` slice (operators see deterministic by-name iteration
/// in the JSON report).
fn run_via_scheduler(
    repos: &[(String, PathBuf)],
    write: bool,
) -> Result<IndexMap<String, SweepOutcome>, CargoError> {
    use std::sync::Arc;

    use shigoto_budget::{BudgetSpec, BudgetTree};
    use shigoto_dag::Dag;
    use shigoto_emit::{InMemorySink, NullEmitter};
    use shigoto_scheduler::{InProcessScheduler, Scheduler};
    use shigoto_types::{JobKindId, JobPhase, OutputSink};

    use crate::gen_build_job::{GenBuildJob, GEN_BUILD_KIND};

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("fleet-sweep")
        .build()
        .map_err(|e| CargoError::FleetSweep(format!("tokio runtime: {e}")))?;

    rt.block_on(async {
        let scheduler = InProcessScheduler::new("fleet-sweep")
            // shigoto NullEmitter is now a type alias for NullSink<_>; construct
            // it via ::new() (matches shigoto-scheduler's own default emitter).
            .with_emitter(Arc::new(NullEmitter::new()));

        let mut budget = BudgetTree::new();
        budget
            .by_kind
            .insert(JobKindId::new(GEN_BUILD_KIND), BudgetSpec::max_concurrent(DEFAULT_BUDGET));
        scheduler.install_budget(budget).await;

        let sink: Arc<InMemorySink<SweepOutcome>> = Arc::new(InMemorySink::new());
        let sink_for_jobs: Arc<dyn OutputSink<SweepOutcome>> = sink.clone();

        let mut dag = Dag::new();
        let mut id_to_repo: IndexMap<shigoto_types::JobId, String> = IndexMap::new();

        for (repo_name, repo_path) in repos {
            let job = Arc::new(
                GenBuildJob::new(repo_name.clone(), repo_path.clone(), write)
                    .with_output_sink(sink_for_jobs.clone()),
            );
            let id = <GenBuildJob as shigoto_types::Job>::id(&job);
            id_to_repo.insert(id.clone(), repo_name.clone());
            dag.ensure_node(id);
            scheduler.register_job(job).await;
        }

        // Per-tick drain until no Job transitions in a tick. Same
        // termination criterion tend's reconcile uses.
        const MAX_TICKS: usize = 4096;
        for _ in 0..MAX_TICKS {
            let receipt = scheduler.tick(&mut dag).await
                .map_err(|e| CargoError::FleetSweep(format!("scheduler tick: {e}")))?;
            if receipt.transitions_this_tick.is_empty() {
                break;
            }
        }

        let snap = scheduler.snapshot(&dag).await;
        let drained = sink.drain();

        // Map outputs by JobId, then walk repos in input order so
        // the IndexMap honours the operator-facing sort order.
        let mut by_repo: IndexMap<String, SweepOutcome> = IndexMap::new();
        for (job_id, outcome) in drained {
            if let Some(repo_name) = id_to_repo.get(&job_id) {
                by_repo.insert(repo_name.clone(), outcome);
            }
        }

        // Cover Jobs that never produced an Output — typically
        // scheduler-level invocation failures. Surface them with a
        // synthetic Failed outcome so the report covers every repo.
        for (id, repo_name) in &id_to_repo {
            if !by_repo.contains_key(repo_name) {
                let phase = snap.phases.get(id).cloned().unwrap_or(JobPhase::Pending);
                by_repo.insert(
                    repo_name.clone(),
                    SweepOutcome::Failed {
                        category: FailureCategory::OtherCargoError,
                        detail: format!("Job terminated without output (phase = {phase:?})"),
                        elapsed_ms: 0,
                    },
                );
            }
        }

        // Re-sort outcomes by the input repo order (deterministic).
        let mut sorted: IndexMap<String, SweepOutcome> = IndexMap::new();
        for (repo_name, _) in repos {
            if let Some(outcome) = by_repo.shift_remove(repo_name) {
                sorted.insert(repo_name.clone(), outcome);
            }
        }
        Ok(sorted)
    })
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
