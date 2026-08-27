//! `bounded` — the ONE sanctioned way for gen to run a subprocess.
//!
//! ── ★ THE CLASS THIS EXISTS TO ELIMINATE ─────────────────────────────
//! `Command::output()` waits forever. There is no bound, no signal, and
//! no way for a caller to tell "working" from "wedged" — so a hung child
//! is indistinguishable from a slow one, and gen simply never returns.
//!
//! Measured 2026-08-27, and this module is the fix for exactly it: a
//! `gen build` in `pangea-operator` sat for **20 minutes at 0.0% CPU**
//! with its log frozen on the same three lines. Underneath were **six
//! orphaned `git index-pack` processes**, each stalled mid-transfer at
//! 0% CPU, from the git-dep prefetch. Nothing was broken and nothing was
//! progressing. The operator's read of it was, correctly, "this refresh
//! should be quick" — the tool gave them no way to know.
//!
//! Worse, the hang defeated a fallback that already existed:
//! `git_clone_at_rev` tries a shallow fetch and falls back to a full one
//! `if shallow_fetch.is_err()`. A fetch that HANGS is never `Err`, so
//! neither the fallback nor the error path is ever reached.
//!
//! ── HOW THE CLASS IS CLOSED, IN THREE LAYERS ─────────────────────────
//! 1. **At cause** — network commands pass git's own
//!    `http.lowSpeedLimit`/`lowSpeedTime`, so git ABORTS a stalled
//!    transfer itself and returns a normal error. This is the layer that
//!    should fire, and it makes the hang a typed failure rather than
//!    something to be killed from outside.
//! 2. **The backstop** — every child is bounded by a deadline here, for
//!    hangs layer 1 cannot see (a wedged `index-pack` *after* transfer,
//!    a filesystem stall, a paused process). Timeout kills the child and
//!    returns [`ExecError::TimedOut`], naming the program and the bound.
//! 3. **Retry** — a bounded failure is retryable, which is the whole
//!    point of bounding it. Transient network faults now recover instead
//!    of wedging a fleet-wide lock refresh.
//!
//! ── WHY THE PIPES ARE DRAINED ON THREADS ─────────────────────────────
//! The obvious implementation — `spawn()`, poll `try_wait()`, then read
//! the pipes — deadlocks whenever a child writes more than a pipe buffer
//! (~64 KiB): the child blocks on write, so it never exits, so
//! `try_wait()` never reports, so we never read. It would trade a rare
//! network hang for a reliable output-volume hang. Two reader threads
//! drain both pipes continuously, so the child can always make progress.
//!
//! ── SCOPE ────────────────────────────────────────────────────────────
//! An unbounded `.output()`/`.status()`/`.wait()` outside this module is
//! a gate violation, enforced by
//! `gen-cargo/tests/no_unbounded_subprocess.rs` against a PENDING ledger
//! that may only shrink. Note the gate targets the blocking WAIT, not
//! `Command::new` — building a command is harmless, and this module must
//! build them itself.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How the child ended, when it did not end well.
#[derive(Debug, thiserror::Error)]
pub enum ExecError {
    #[error("spawn `{program}`: {source}")]
    Spawn {
        program: String,
        #[source]
        source: std::io::Error,
    },
    /// The bound fired. The child was killed and reaped.
    ///
    /// Distinct from `Failed` on purpose: a timeout says nothing about
    /// whether the work was valid, only that it did not finish, so it is
    /// the one outcome that is unconditionally worth retrying.
    #[error(
        "`{program} {args}` exceeded its {bound_secs}s bound and was killed \
         (this is gen's hang backstop — the command made no progress)"
    )]
    TimedOut {
        program: String,
        args: String,
        bound_secs: u64,
    },
    #[error("`{program} {args}` exited {status}: {stderr}")]
    Failed {
        program: String,
        args: String,
        status: String,
        stderr: String,
    },
}

impl ExecError {
    /// Is retrying this plausibly useful?
    ///
    /// A timeout always is. A clean non-zero exit usually is NOT — "rev
    /// not found" does not improve on the second attempt — so retrying
    /// those is opt-in per call site rather than the default, to avoid
    /// turning a fast deterministic failure into a slow one.
    #[must_use]
    pub fn is_timeout(&self) -> bool {
        matches!(self, ExecError::TimedOut { .. })
    }
}

/// A finished child's captured streams.
#[derive(Debug)]
pub struct Output {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// A subprocess runner with a mandatory deadline.
///
/// The deadline is a constructor argument rather than an option with a
/// default, because "I forgot to set a timeout" is precisely the defect
/// this module exists to make unrepresentable.
#[derive(Debug, Clone, Copy)]
pub struct Bounded {
    deadline: Duration,
    attempts: u32,
    retry_failures: bool,
}

impl Bounded {
    #[must_use]
    pub fn new(deadline: Duration) -> Self {
        Self {
            deadline,
            attempts: 1,
            retry_failures: false,
        }
    }

    /// Total attempts, not extra ones. `attempts(1)` is no retry.
    #[must_use]
    pub fn attempts(mut self, n: u32) -> Self {
        self.attempts = n.max(1);
        self
    }

    /// Also retry a clean non-zero exit — for network commands, where a
    /// connection reset is reported as a normal failure rather than a
    /// timeout.
    #[must_use]
    pub fn retry_failures(mut self, yes: bool) -> Self {
        self.retry_failures = yes;
        self
    }

    /// Run to completion, or kill at the deadline.
    ///
    /// `build` is a closure rather than a `Command` because a `Command`
    /// cannot be cloned, and a retry needs a fresh one.
    ///
    /// # Errors
    /// [`ExecError`] on spawn failure, timeout, or non-zero exit.
    pub fn run<F>(&self, build: F) -> Result<Output, ExecError>
    where
        F: Fn() -> Command,
    {
        let mut last: Option<ExecError> = None;
        for attempt in 0..self.attempts {
            match self.run_once(&build) {
                Ok(o) => return Ok(o),
                Err(e) => {
                    let retryable = e.is_timeout() || self.retry_failures;
                    if !retryable || attempt + 1 == self.attempts {
                        return Err(e);
                    }
                    // Linear backoff: these are seconds-scale network
                    // operations, so exponential adds latency without
                    // adding much decorrelation.
                    std::thread::sleep(Duration::from_millis(500 * u64::from(attempt + 1)));
                    last = Some(e);
                }
            }
        }
        Err(last.expect("attempts >= 1 guarantees at least one error here"))
    }

    fn run_once<F>(&self, build: &F) -> Result<Output, ExecError>
    where
        F: Fn() -> Command,
    {
        let mut cmd = build();
        let program = cmd.get_program().to_string_lossy().into_owned();
        let args = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" ");

        let mut child = cmd
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|source| ExecError::Spawn {
                program: program.clone(),
                source,
            })?;

        // Drain BOTH pipes concurrently. See the module header: reading
        // after the wait deadlocks on any child that outproduces a pipe
        // buffer.
        let mut out_h = child.stdout.take();
        let mut err_h = child.stderr.take();
        let out_t = std::thread::spawn(move || {
            let mut b = Vec::new();
            if let Some(h) = out_h.as_mut() {
                let _ = h.read_to_end(&mut b);
            }
            b
        });
        let err_t = std::thread::spawn(move || {
            let mut b = Vec::new();
            if let Some(h) = err_h.as_mut() {
                let _ = h.read_to_end(&mut b);
            }
            b
        });

        let start = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(s)) => break s,
                Ok(None) => {}
                Err(source) => {
                    return Err(ExecError::Spawn { program, source });
                }
            }
            if start.elapsed() >= self.deadline {
                // Kill, then REAP. Skipping the wait leaves a zombie and,
                // worse, leaves the reader threads blocked on pipes that
                // never close — which would hang the very function whose
                // job is to not hang.
                let _ = child.kill();
                let _ = child.wait();
                let _ = out_t.join();
                let _ = err_t.join();
                return Err(ExecError::TimedOut {
                    program,
                    args,
                    bound_secs: self.deadline.as_secs(),
                });
            }
            std::thread::sleep(Duration::from_millis(50));
        };

        let stdout = out_t.join().unwrap_or_default();
        let stderr = err_t.join().unwrap_or_default();
        if !status.success() {
            return Err(ExecError::Failed {
                program,
                args,
                status: status.to_string(),
                stderr: String::from_utf8_lossy(&stderr).trim().to_string(),
            });
        }
        Ok(Output { stdout, stderr })
    }
}

/// git flags that make git abort a stalled transfer ITSELF.
///
/// Layer 1 of the three above, and the one that should normally fire:
/// below 1 KiB/s for 30s, git errors out instead of sitting at 0% CPU
/// forever. Applied to every network-touching git invocation.
#[must_use]
pub fn git_stall_guard() -> [&'static str; 4] {
    [
        "-c",
        "http.lowSpeedLimit=1024",
        "-c",
        "http.lowSpeedTime=30",
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hanging_child_is_killed_at_the_deadline() {
        let b = Bounded::new(Duration::from_millis(300));
        let started = Instant::now();
        let err = b
            .run(|| {
                let mut c = Command::new("sleep");
                c.arg("30");
                c
            })
            .expect_err("a 30s sleep must not survive a 300ms bound");
        assert!(err.is_timeout(), "{err}");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the bound must actually bound: took {:?}",
            started.elapsed()
        );
        assert!(err.to_string().contains("hang backstop"), "{err}");
    }

    /// ★ THE DEADLOCK THIS MODULE IS SHAPED TO AVOID.
    ///
    /// A child writing far more than a pipe buffer must still complete.
    /// With the naive "wait, then read" ordering this test hangs forever
    /// rather than failing — which is why the pipes are drained on
    /// threads.
    #[test]
    fn a_child_that_outproduces_the_pipe_buffer_does_not_deadlock() {
        let b = Bounded::new(Duration::from_secs(30));
        let out = b
            .run(|| {
                let mut c = Command::new("sh");
                // ~2 MiB, far past any pipe buffer.
                c.args(["-c", "yes abcdefghijklmnopqrstuvwxyz | head -c 2000000"]);
                c
            })
            .expect("large output must not deadlock");
        assert_eq!(out.stdout.len(), 2_000_000);
    }

    #[test]
    fn a_clean_failure_is_typed_and_carries_stderr() {
        let b = Bounded::new(Duration::from_secs(10));
        let err = b
            .run(|| {
                let mut c = Command::new("sh");
                c.args(["-c", "echo boom >&2; exit 3"]);
                c
            })
            .expect_err("exit 3 is a failure");
        assert!(!err.is_timeout());
        assert!(err.to_string().contains("boom"), "{err}");
    }

    /// A non-zero exit must NOT be retried by default — a deterministic
    /// failure retried N times is just a slower deterministic failure.
    #[test]
    fn failures_are_not_retried_unless_asked() {
        let b = Bounded::new(Duration::from_secs(10)).attempts(3);
        let started = Instant::now();
        let _ = b.run(|| {
            let mut c = Command::new("sh");
            c.args(["-c", "exit 1"]);
            c
        });
        assert!(
            started.elapsed() < Duration::from_millis(900),
            "default retry policy must not sleep between deterministic failures"
        );
    }

    #[test]
    fn spawning_a_missing_program_is_typed_not_a_panic() {
        let b = Bounded::new(Duration::from_secs(5));
        let err = b
            .run(|| Command::new("gen-no-such-binary-anywhere"))
            .expect_err("missing program");
        assert!(matches!(err, ExecError::Spawn { .. }), "{err}");
    }

    #[test]
    fn the_stall_guard_sets_both_halves() {
        // Either flag alone does nothing: a limit with no time never
        // triggers, and a time with no limit has no threshold.
        let g = git_stall_guard();
        assert!(g.iter().any(|f| f.starts_with("http.lowSpeedLimit=")));
        assert!(g.iter().any(|f| f.starts_with("http.lowSpeedTime=")));
    }
}
