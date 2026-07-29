//! **The non-vacuous proof for the D2 freshness tie.**
//!
//! This file exists because a gate that has never been observed to fail may
//! be checking nothing (`theory/UNREPRESENTABILITY.md` §II.3). The D2 tie was
//! green for years over a class of change it structurally could not see, and
//! reading its source found nothing (it read correct) and running it found
//! nothing (it was green). The only thing that catches that is **deliberately
//! breaking the guarded thing and watching for a failure that does not come.**
//!
//! So: construct a real change that alters the resolved FEATURE SET while
//! leaving `Cargo.lock` byte-identical, and demand the tie go red.
//!
//! Deliberately written against only the API that existed BEFORE the tie was
//! widened, so the identical file compiles in both trees: run it on the
//! pre-fix tree and it FAILS (the verdict is `Fresh`); run it here and it
//! passes. That before/after pair is the evidence, not the green tick.
//!
//! The fixture-validity gate is load-bearing in the other direction too: if
//! `Cargo.lock` ever moves between the two states, this test FAILS rather
//! than passing for the wrong reason — a fixture that does not reproduce the
//! defect cannot be allowed to certify the fix.

use std::fs;
use std::path::Path;
use std::process::Command;

use gen_cargo::build_spec;
use gen_cargo::gen_delta::{self, DeltaFreshness, GenDeltaArtifact};

/// A one-crate workspace whose ONLY difference between states is which
/// features `default` activates. No dependency is added or removed, so
/// cargo has nothing new to resolve and `Cargo.lock` cannot move.
fn manifest(default_activates_extra: bool) -> String {
    let mut s = String::from(
        "[package]\n\
         name = \"d2-tie-fixture\"\n\
         version = \"0.1.0\"\n\
         edition = \"2024\"\n\n\
         [features]\n\
         default = ",
    );
    s.push_str(if default_activates_extra {
        "[\"extra\"]"
    } else {
        "[]"
    });
    s.push_str("\nextra = []\n");
    s
}

/// Exactly what `gen build` does: emit BOTH `Cargo.build-spec.json` and
/// `Cargo.gen.lock`. Writing only the delta would leave `check_freshness`
/// with no spec to consult, and the producer assertion below would then pass
/// for the wrong reason (`MissingSpec` also `needs_regen()`) — a guard
/// reporting green over a subject it never examined.
fn gen_build(root: &Path) {
    build_spec::generate_multi_target_and_write(root).expect("gen build");
}

fn lockfile(root: &Path) {
    // Offline: the fixture has no dependencies, so no registry is needed.
    let out = Command::new("cargo")
        .args(["generate-lockfile", "--offline"])
        .current_dir(root)
        .output()
        .expect("cargo generate-lockfile");
    assert!(
        out.status.success(),
        "cargo generate-lockfile failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn a_feature_set_change_that_leaves_cargo_lock_identical_is_caught() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();

    // ── State A: `default = ["extra"]` ────────────────────────────────
    fs::write(root.join("Cargo.toml"), manifest(true)).unwrap();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/lib.rs"), "pub fn f() {}\n").unwrap();
    lockfile(root);
    gen_build(root);

    let lock_a = fs::read(root.join("Cargo.lock")).unwrap();
    let delta_a = fs::read_to_string(root.join("Cargo.gen.lock")).unwrap();
    let verdict_a = gen_delta::confirm_freshness(root);
    eprintln!("[A] verdict: {verdict_a:?}");
    assert!(
        matches!(verdict_a, DeltaFreshness::Fresh { .. }),
        "a just-generated delta must be fresh against its own workspace, got {verdict_a:?}"
    );

    // ── State B: `default = []` — feature-set-only change ─────────────
    fs::write(root.join("Cargo.toml"), manifest(false)).unwrap();
    lockfile(root);

    // FIXTURE-VALIDITY GATE. The whole proof rests on this: if the lock
    // moved, the old tie would have caught the change on the lock half and
    // this test would certify nothing.
    let lock_b = fs::read(root.join("Cargo.lock")).unwrap();
    assert_eq!(
        lock_a,
        lock_b,
        "FIXTURE INVALID: Cargo.lock moved between the two states, so this \
         test does not reproduce the defect and cannot certify the fix"
    );
    eprintln!("[B] Cargo.lock byte-identical: YES ({} bytes)", lock_b.len());

    // The delta REALLY IS different for state B — i.e. the committed
    // artifact from state A genuinely no longer describes this build. Without
    // this, "the tie went red" would not prove the tie was right to.
    let spec_b = build_spec::generate_multi_target(root).expect("spec B");
    let delta_b = gen_delta::GenDelta::distill(&spec_b)
        .expect("distill B")
        .to_json()
        .expect("serialize B")
        + "\n";
    assert_ne!(
        delta_a, delta_b,
        "the feature flip must actually change the delta, else there is \
         nothing for the tie to catch"
    );

    // ── The verdict under test ────────────────────────────────────────
    let verdict_b = gen_delta::confirm_freshness(root);
    eprintln!("[B] verdict (state-A delta still committed): {verdict_b:?}");
    assert!(
        !matches!(verdict_b, DeltaFreshness::Fresh { .. }),
        "THE DEFECT: the committed delta describes state A, the workspace is \
         in state B, and the tie reported {verdict_b:?}"
    );
    assert!(verdict_b.is_stale(), "must classify as stale");
    // Must gate in BOTH modes — a present, provably-wrong delta is never
    // tolerated, not even by the lenient `--if-present` fleet check.
    assert!(
        verdict_b.gates_failure(true) && verdict_b.gates_failure(false),
        "a provably-stale delta must fail the gate in strict AND --if-present \
         mode, got {verdict_b:?}"
    );
    // (The itemized `ManifestDrift` shape is asserted structurally in
    // `d2_tie_widened_subject_set.rs`, which is free to name post-fix API.)

    // The PRODUCER fast-path must agree, or `gen build --if-stale` would
    // refuse to regenerate and the gate above would be permanently
    // unsatisfiable through it.
    let producer = build_spec::check_freshness(root);
    eprintln!("[B] producer check_freshness: {producer:?}");
    assert!(
        producer.needs_regen(),
        "gen build --if-stale must regenerate for a manifest-only change, \
         got {producer:?}"
    );

    // ── The other vacuity direction: the gate must go GREEN again ─────
    // A gate that is red unconditionally is as useless as one that is green
    // unconditionally.
    gen_build(root);
    let verdict_c = gen_delta::confirm_freshness(root);
    eprintln!("[C] verdict after `gen build`: {verdict_c:?}");
    assert!(
        matches!(verdict_c, DeltaFreshness::Fresh { .. }),
        "after regeneration the tie must go green again, got {verdict_c:?}"
    );
}
