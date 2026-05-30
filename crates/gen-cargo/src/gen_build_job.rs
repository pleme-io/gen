//! `GenBuildJob` — typed shigoto Job for "regenerate one repo's
//! BuildSpec." The fourth shigoto-native Job kind in the fleet
//! (after tend's `pull-repo` / `status-repo` / `fetch-repo` /
//! `sync-repo`); first in the gen domain.
//!
//! One job = one repo on disk. `execute_body` defers to the
//! existing sync `sweep_one_sync` helper through
//! `tokio::task::spawn_blocking` so the cargo-metadata-heavy work
//! doesn't stall the scheduler's async runtime. The `Output` is a
//! `SweepOutcome` — the same typed-enum the legacy sequential loop
//! returned, now flowing through an `InMemorySink<SweepOutcome>`
//! that the post-run drain feeds into the `SweepReport`.
//!
//! Adding a new gen Job kind = adding one more file like this one
//! + one `KIND` constant + a `BudgetSpec::max_concurrent` entry in
//! the consumer's `BudgetTree`. Substrate compounding by
//! construction.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use shigoto_types::{JobScope, JobSubject, OutputSink, RecordingJob};
use thiserror::Error;

use crate::fleet_sweep::SweepOutcome;

/// Canonical kind id for every GenBuildJob. Used by the consumer's
/// `BudgetTree` to cap concurrent invocations + by emitters to
/// route transitions.
pub const GEN_BUILD_KIND: &str = "gen.build-spec";

/// One repo's regen request. Cloned per-tick by the scheduler.
#[derive(Clone)]
pub struct GenBuildJob {
    pub repo_name: String,
    pub repo_path: PathBuf,
    pub write: bool,
    output_sink: Option<Arc<dyn OutputSink<SweepOutcome>>>,
}

impl std::fmt::Debug for GenBuildJob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GenBuildJob")
            .field("repo_name", &self.repo_name)
            .field("repo_path", &self.repo_path)
            .field("write", &self.write)
            .field("output_sink", &self.output_sink.as_ref().map(|_| "<sink>"))
            .finish()
    }
}

impl GenBuildJob {
    #[must_use]
    pub fn new(
        repo_name: impl Into<String>,
        repo_path: impl Into<PathBuf>,
        write: bool,
    ) -> Self {
        Self {
            repo_name: repo_name.into(),
            repo_path: repo_path.into(),
            write,
            output_sink: None,
        }
    }

    #[must_use]
    pub fn with_output_sink(mut self, sink: Arc<dyn OutputSink<SweepOutcome>>) -> Self {
        self.output_sink = Some(sink);
        self
    }
}

#[derive(Debug, Error)]
pub enum GenBuildError {
    #[error("gen-build-spec invocation failed: {0}")]
    Invocation(String),
}

#[async_trait]
impl RecordingJob for GenBuildJob {
    type Output = SweepOutcome;
    type Error = GenBuildError;
    const KIND: &'static str = GEN_BUILD_KIND;

    fn scope(&self) -> JobScope {
        // fleet-sweep is a fleet-wide operation; every Job shares
        // the same anchor scope. Per-kind budget caps concurrency.
        JobScope::Workspace("fleet-sweep".to_string())
    }

    fn subject(&self) -> JobSubject {
        JobSubject::Repo(self.repo_name.clone())
    }

    fn output_sink(&self) -> Option<&Arc<dyn OutputSink<Self::Output>>> {
        self.output_sink.as_ref()
    }

    async fn execute_body(&self) -> Result<SweepOutcome, GenBuildError> {
        let path = self.repo_path.clone();
        let write = self.write;
        tokio::task::spawn_blocking(move || sweep_one_sync(&path, write))
            .await
            .map_err(|join_err| GenBuildError::Invocation(format!("join error: {join_err}")))
    }
}

/// Sync per-repo body. Returns a typed `SweepOutcome` even on
/// upstream cargo-metadata errors — the Job's `Result` carries
/// scheduler-level invocation failure (tokio panic, join error);
/// every cargo-state outcome lives inside `SweepOutcome` so the
/// post-run report sees uniform shape.
pub(crate) fn sweep_one_sync(repo: &Path, write: bool) -> SweepOutcome {
    use std::time::Instant;

    use crate::build_spec;
    use crate::fleet_sweep::{FailureCategory, SkipReason};

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
        build_spec::generate_and_write_if_stale(repo).map(|(_freshness, path)| {
            std::fs::metadata(&path)
                .map(|m| m.len() as usize)
                .unwrap_or(0)
        })
    } else {
        build_spec::generate(repo).map(|spec| {
            serde_json::to_string(&spec).map(|s| s.len()).unwrap_or(0)
        })
    };
    let elapsed_ms = started.elapsed().as_millis() as u64;
    match spec_result {
        Ok(spec_bytes) => SweepOutcome::Ok {
            spec_bytes,
            elapsed_ms,
        },
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
    use shigoto_types::{Job, JobKindId};

    #[test]
    fn kind_id_is_canonical() {
        let job = GenBuildJob::new("foo", "/tmp/foo", false);
        let id = <GenBuildJob as Job>::id(&job);
        assert_eq!(id.kind, JobKindId::new(GEN_BUILD_KIND));
    }

    #[test]
    fn missing_cargo_toml_yields_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let outcome = sweep_one_sync(tmp.path(), false);
        assert!(matches!(
            outcome,
            SweepOutcome::Skipped {
                reason: crate::fleet_sweep::SkipReason::NoCargoToml,
            }
        ));
    }
}
