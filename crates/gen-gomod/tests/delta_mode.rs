//! Tests for the typed retirement of the `Go.gen.lock` producer.
//!
//! The load-bearing assertion is `retired_policy_writes_nothing`: it observes a
//! WRITE COUNT through a mock seam rather than the absence of a file, because
//! "no file appeared" is also what a broken test produces.
//!
//! The compile-time half of this defuse cannot be asserted from here by
//! construction — a test that fails to compile is not a test. It was recorded
//! instead as a measured receipt: before this change,
//! `gen_gomod::gen_delta::write_gen_delta` was publicly reachable and a
//! reference to it COMPILED; after, the same file fails with
//! `error[E0603]: module 'gen_delta' is private`. See the commit message.

use gen_gomod::delta_mode::{
    DeltaEnv, DeltaMode, DeltaOutcome, DeltaPolicy, RetirementReason, apply,
};
use std::path::{Path, PathBuf};

#[derive(Default)]
struct MockDeltaEnv {
    writes: Vec<(PathBuf, String)>,
}

impl DeltaEnv for MockDeltaEnv {
    fn write(&mut self, path: &Path, contents: &str) -> std::io::Result<()> {
        self.writes.push((path.to_path_buf(), contents.to_string()));
        Ok(())
    }
}

#[test]
fn shipped_policy_is_retired() {
    assert!(DeltaPolicy::RETIRED.is_retired());
    assert_eq!(
        DeltaPolicy::RETIRED.mode,
        DeltaMode::Retired(RetirementReason::NoConsumer)
    );
}

#[test]
fn retired_policy_mints_no_witness() {
    // The witness is the only key to the writer; retired must not produce one.
    assert!(DeltaPolicy::RETIRED.activate().is_none());
}

#[test]
fn active_policy_mints_a_witness() {
    // Proves the refusal is conditional, not hard-wired — without this, an
    // `activate` that always returned None would pass every other test.
    let p = DeltaPolicy {
        mode: DeltaMode::Active,
    };
    assert!(p.activate().is_some());
}

#[test]
fn retired_policy_writes_nothing() {
    let mut env = MockDeltaEnv::default();
    let out = apply(DeltaPolicy::RETIRED, &mut env, Path::new("/tmp/x"), || {
        panic!("renderer must not run while retired");
    })
    .expect("retired apply is not an error");

    assert_eq!(
        out,
        DeltaOutcome::Skipped {
            reason: RetirementReason::NoConsumer
        }
    );
    // The assertion that matters: an observed write count, not a missing file.
    assert_eq!(env.writes.len(), 0, "retired policy must not write");
}

#[test]
fn active_policy_writes_once() {
    let mut env = MockDeltaEnv::default();
    let p = DeltaPolicy {
        mode: DeltaMode::Active,
    };
    let out = apply(p, &mut env, Path::new("/tmp/x"), || Ok("{}".to_string()))
        .expect("active apply succeeds");

    assert!(matches!(out, DeltaOutcome::Wrote { .. }));
    assert_eq!(env.writes.len(), 1, "active policy writes exactly once");
    assert!(env.writes[0].0.ends_with("Go.gen.lock"));
}

#[test]
fn declared_spec_agrees_with_the_code() {
    // The (def…) form and DeltaPolicy::RETIRED must not drift.
    let spec = include_str!("../specs/go-delta.lisp");
    assert!(
        spec.contains(":mode \"retired\""),
        "spec must declare retired"
    );
    assert!(
        spec.contains(":reason \"no-consumer\""),
        "spec must declare the reason"
    );
    assert_eq!(
        DeltaPolicy::RETIRED.mode,
        DeltaMode::Retired(RetirementReason::NoConsumer),
        "code must match the declaration"
    );
}
