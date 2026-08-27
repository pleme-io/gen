//! Structural tests for the widened D2 subject set — the arms and the
//! witnesses `d2_tie_feature_set_drift.rs` deliberately cannot name (that
//! file is kept compilable against the pre-fix tree so it can serve as the
//! before/after evidence).
//!
//! Every test here is a **deliberately broken input** run against the guard,
//! per the standing rule that a PR adding or modifying a gate records a red
//! run against one.

use std::fs;
use std::path::Path;

use gen_cargo::gen_delta::{
    DELTA_SCHEMA_VERSION, DeltaFreshness, GenDeltaArtifact, confirm_freshness,
};

fn tmpdir(tag: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static C: AtomicU64 = AtomicU64::new(0);
    let n = C.fetch_add(1, Ordering::Relaxed);
    let mut name = String::from("gen-d2-tie-");
    name.push_str(tag);
    name.push('-');
    name.push_str(&std::process::id().to_string());
    name.push('-');
    name.push_str(&n.to_string());
    let p = std::env::temp_dir().join(name);
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

const HEX: &[u8; 16] = b"0123456789abcdef";

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    let d = h.finalize();
    let mut s = String::with_capacity(d.len() * 2);
    for b in &d {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0x0f) as usize] as char);
    }
    s
}

/// A minimal `Cargo.gen.lock` carrying exactly the two tie halves. The
/// offline confirm consults nothing else, so this is the honest fixture.
fn write_delta(dir: &Path, lock_sha: &str, manifests: &[(&str, &str)]) {
    let mut body = String::from("{ \"schema_version\": ");
    body.push_str(&DELTA_SCHEMA_VERSION.to_string());
    body.push_str(", \"cargo_lock_sha256\": \"");
    body.push_str(lock_sha);
    body.push('"');
    if !manifests.is_empty() {
        body.push_str(", \"manifest_sha256\": {");
        for (i, (path, sha)) in manifests.iter().enumerate() {
            if i > 0 {
                body.push(',');
            }
            body.push_str(" \"");
            body.push_str(path);
            body.push_str("\": \"");
            body.push_str(sha);
            body.push('"');
        }
        body.push_str(" }");
    }
    body.push_str(" }");
    fs::write(dir.join("Cargo.gen.lock"), body).unwrap();
}

/// The delta schema literal is a forcing function, the same shape
/// `strict_pipeline::schema_is_v12_const` gives the full spec — and the
/// delta had none until now. A delta bump invalidates every committed
/// `Cargo.gen.lock` in the fleet, so it must be an explicit reviewed act
/// rather than something that rides along in a diff. Comparing against the
/// constant alone would be tautological.
#[test]
fn delta_schema_is_v2() {
    assert_eq!(
        DELTA_SCHEMA_VERSION, 2,
        "DELTA_SCHEMA_VERSION must be 2 (v2 adds `manifest_sha256`, the \
         declaration half of the D2 tie)"
    );
}

#[test]
fn both_halves_matching_is_fresh_and_carries_its_subject_set_size() {
    let dir = tmpdir("fresh");
    let lock = b"# lock\n";
    let root_manifest = b"[workspace]\nmembers = []\n";
    fs::write(dir.join("Cargo.lock"), lock).unwrap();
    fs::write(dir.join("Cargo.toml"), root_manifest).unwrap();
    write_delta(
        &dir,
        &sha256_hex(lock),
        &[("Cargo.toml", &sha256_hex(root_manifest))],
    );
    assert_eq!(
        confirm_freshness(&dir),
        DeltaFreshness::Fresh {
            cargo_lock_sha256: sha256_hex(lock),
            // The witness: this verdict is about exactly one manifest.
            manifests_verified: 1,
        }
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_manifest_edit_with_an_unmoved_lock_is_itemized_drift() {
    let dir = tmpdir("drift");
    let lock = b"# lock\n";
    fs::write(dir.join("Cargo.lock"), lock).unwrap();
    fs::write(dir.join("Cargo.toml"), b"[features]\ndefault = [\"a\"]\n").unwrap();
    let recorded = sha256_hex(b"[features]\ndefault = [\"a\"]\n");
    write_delta(&dir, &sha256_hex(lock), &[("Cargo.toml", &recorded)]);
    // Feature flip. The lock is untouched.
    fs::write(dir.join("Cargo.toml"), b"[features]\ndefault = []\n").unwrap();

    match confirm_freshness(&dir) {
        DeltaFreshness::ManifestDrift { changed, .. } => {
            assert_eq!(changed.len(), 1);
            assert_eq!(changed[0].path, "Cargo.toml");
            assert_eq!(changed[0].expected, recorded);
            assert_eq!(
                changed[0].actual,
                Some(sha256_hex(b"[features]\ndefault = []\n"))
            );
        }
        other => panic!("expected ManifestDrift, got {other:?}"),
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn manifest_drift_gates_in_both_strict_and_if_present_mode() {
    // A present, provably-wrong delta is never a baseline-debt case.
    let drift = DeltaFreshness::ManifestDrift {
        cargo_lock_sha256: "a".into(),
        changed: vec![gen_cargo::manifest_tie::ManifestDrift {
            path: "Cargo.toml".into(),
            expected: "a".into(),
            actual: Some("b".into()),
        }],
    };
    assert!(drift.is_stale());
    assert!(drift.gates_failure(true));
    assert!(drift.gates_failure(false));
}

#[test]
fn a_vanished_manifest_is_drift_never_a_silent_skip() {
    let dir = tmpdir("gone");
    let lock = b"# lock\n";
    fs::write(dir.join("Cargo.lock"), lock).unwrap();
    // The delta records a manifest that is not on disk at all.
    write_delta(
        &dir,
        &sha256_hex(lock),
        &[("crates/gone/Cargo.toml", &sha256_hex(b"whatever"))],
    );
    match confirm_freshness(&dir) {
        DeltaFreshness::ManifestDrift { changed, .. } => {
            assert_eq!(changed.len(), 1);
            assert_eq!(changed[0].actual, None, "fail-closed on a missing subject");
        }
        other => panic!("a missing recorded manifest must be drift, got {other:?}"),
    }
    let _ = fs::remove_dir_all(&dir);
}

// ── The migration arm ────────────────────────────────────────────────────

#[test]
fn a_schema_v1_delta_reads_as_untied_manifests_not_as_fresh() {
    let dir = tmpdir("v1");
    let lock = b"# lock\n";
    fs::write(dir.join("Cargo.lock"), lock).unwrap();
    fs::write(dir.join("Cargo.toml"), b"[workspace]\n").unwrap();
    // Exactly the shape of the 413 committed artifacts in the fleet today:
    // schema_version 1, a lock tie, no manifest digests.
    let mut body = String::from("{ \"schema_version\": 1, \"cargo_lock_sha256\": \"");
    body.push_str(&sha256_hex(lock));
    body.push_str("\" }");
    fs::write(dir.join("Cargo.gen.lock"), body).unwrap();

    let v = confirm_freshness(&dir);
    assert_eq!(
        v,
        DeltaFreshness::UntiedManifests {
            cargo_lock_sha256: sha256_hex(lock)
        },
        "a v1 delta must report what it cannot prove — never round up to Fresh"
    );
    assert!(v.is_stale(), "unprovable is not fresh");
    // Baseline-debt shape: strict `gen confirm` reports it; the fleet's
    // lenient `--if-present` nix-flake-check tolerates it so the gate is
    // adoptable the day it lands.
    assert!(v.gates_failure(true), "strict mode must report the debt");
    assert!(
        !v.gates_failure(false),
        "--if-present must tolerate the pre-widening baseline, else 406 repos \
         go red on day one"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn distill_refuses_a_spec_with_no_manifest_digests() {
    // The producer side of the same invariant: a v2 delta that ADVERTISES the
    // widened tie while carrying an empty subject set for half of it is a
    // guard over zero subjects. It cannot be emitted.
    let raw = include_str!("../src/testdata/v10-build-spec.json");
    let mut spec: gen_cargo::build_spec::BuildSpec = serde_json::from_str(raw).unwrap();
    spec.manifest_sha256.clear();
    let err = gen_cargo::gen_delta::GenDelta::distill(&spec)
        .expect_err("a spec with no manifest digests must not yield a v2 delta");
    assert!(
        matches!(err, gen_cargo::gen_delta::GenDeltaError::NoManifestDigests),
        "got {err:?}"
    );
}
