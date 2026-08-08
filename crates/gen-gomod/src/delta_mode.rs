//! `delta_mode` — the typed retirement of gen-gomod's `Go.gen.lock` producer.
//!
//! # Why this module exists
//!
//! `write_gen_delta` is a complete, tested emitter with **zero call sites**.
//! `gen-cargo`'s sibling has one; this one has none. So no `Go.gen.lock` has
//! ever been produced: 0 exist fleet-wide against a 425-file `Cargo.gen.lock`
//! control (measured 2026-08-08).
//!
//! That is not harmless. Until this module landed, `gen_delta` was a `pub mod`
//! and `write_gen_delta` a `pub fn`, so the shortest path from "this looks
//! unfinished" to "shipped" was one line. And the consumer half defaulted 17
//! keys with bare `or`, so wiring that line would have produced a
//! **zero-package build spec behind a green freshness verdict** rather than an
//! error. The consumer was closed in `substrate@e0fa9a4`; this is the producer
//! half of the same defuse.
//!
//! # MODULARIZE, DON'T DELETE
//!
//! Nothing is removed. The emitter is retained, compiled and tested, and
//! reviving it is a declaration change — `:mode "active"` in
//! `specs/go-delta.lisp` — not an archaeology exercise. What changes is that
//! it can no longer be reached *by accident*: `write_gen_delta` now requires
//! an [`ActiveDelta`] witness, and the only constructor for that witness reads
//! the declared mode.
//!
//! # Tier
//!
//! **parse-time-rejected**, and the honest limit is worth stating: a caller
//! inside this crate can still construct the witness, because `ActiveDelta`'s
//! field is `pub(crate)`. What is now impossible is an *external* caller
//! reaching the writer at all (`gen_delta` is `pub(crate)`), and an *internal*
//! caller obtaining a witness without going through [`DeltaPolicy::activate`],
//! which refuses while the declared mode is retired. That is a real barrier,
//! not a compile-time proof over the whole program — do not round it up to
//! truly-unrep.

use std::path::Path;

/// What the declaration says should happen to the delta artifact.
///
/// Closed on purpose. A new mode is a new variant plus a match arm everywhere,
/// which is the point: "what do we do about `Go.gen.lock`" should never be
/// answerable by a boolean somebody flips.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeltaMode {
    /// Emit `Go.gen.lock` alongside the build spec.
    Active,
    /// Do not emit. The emitter stays compiled and tested.
    Retired(RetirementReason),
}

/// Why a producer is retired. Carrying the reason in the type keeps it from
/// decaying into a comment that outlives its truth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetirementReason {
    /// The artifact has no consumer that reads it, so emitting it would
    /// create a freshness obligation nothing benefits from.
    NoConsumer,
    /// The consumer exists but cannot yet verify the artifact's shape.
    ConsumerCannotVerify,
}

impl RetirementReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoConsumer => "no-consumer",
            Self::ConsumerCannotVerify => "consumer-cannot-verify",
        }
    }
}

/// Proof that emission is permitted. **Cannot be constructed outside this
/// crate**, and inside it only via [`DeltaPolicy::activate`].
#[derive(Debug)]
pub struct ActiveDelta {
    pub(crate) _private: (),
}

/// The declared policy, parsed from `specs/go-delta.lisp`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeltaPolicy {
    pub mode: DeltaMode,
}

impl DeltaPolicy {
    /// The shipped policy. Mirrors `specs/go-delta.lisp`; `spec_parity` asserts
    /// the two agree, so the Lisp declaration cannot drift from this default.
    pub const RETIRED: Self = Self {
        mode: DeltaMode::Retired(RetirementReason::NoConsumer),
    };

    /// The ONLY way to obtain the witness. Returns `None` while retired, so a
    /// caller cannot reach the writer without first changing the declaration.
    #[must_use]
    pub fn activate(self) -> Option<ActiveDelta> {
        match self.mode {
            DeltaMode::Active => Some(ActiveDelta { _private: () }),
            DeltaMode::Retired(_) => None,
        }
    }

    #[must_use]
    pub fn is_retired(self) -> bool {
        matches!(self.mode, DeltaMode::Retired(_))
    }
}

/// What a delta run did. `#[must_use]` so a caller cannot drop the outcome and
/// assume something happened — the exact shape that let the producer sit
/// unexamined.
#[derive(Debug, PartialEq, Eq)]
#[must_use]
pub enum DeltaOutcome {
    /// Emission was skipped because the policy is retired.
    Skipped { reason: RetirementReason },
    /// The artifact was written at this path.
    Wrote { path: String },
}

/// The seam. Mockable, so the retirement is testable without touching a disk —
/// and so `retired_policy_writes_nothing` asserts on an observed write count
/// rather than on the absence of a file, which is the weaker claim.
pub trait DeltaEnv {
    /// Write `contents` to `path`.
    fn write(&mut self, path: &Path, contents: &str) -> std::io::Result<()>;
}

/// The real environment: the filesystem.
#[derive(Debug, Default)]
pub struct FsDeltaEnv;

impl DeltaEnv for FsDeltaEnv {
    fn write(&mut self, path: &Path, contents: &str) -> std::io::Result<()> {
        std::fs::write(path, contents)
    }
}

/// Apply the policy. This is the interpreter half of the triplet: the
/// declaration decides, this executes, and the seam makes the decision
/// observable in a test.
pub fn apply<E: DeltaEnv>(
    policy: DeltaPolicy,
    env: &mut E,
    root: &Path,
    render: impl FnOnce() -> Result<String, crate::gen_delta::GenDeltaError>,
) -> Result<DeltaOutcome, crate::gen_delta::GenDeltaError> {
    match policy.mode {
        DeltaMode::Retired(reason) => Ok(DeltaOutcome::Skipped { reason }),
        DeltaMode::Active => {
            let path = root.join(crate::gen_delta::GoGenDelta::FILENAME);
            let body = render()?;
            env.write(&path, &body)
                .map_err(|source| crate::gen_delta::GenDeltaError::Write {
                    path: path.display().to_string(),
                    source,
                })?;
            Ok(DeltaOutcome::Wrote {
                path: path.display().to_string(),
            })
        }
    }
}
