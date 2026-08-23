//! The compile-fail corpus — the half of this crate's claim that a passing
//! test can never show — plus the guard that keeps the corpus from going
//! vacuous on itself.
//!
//! Every tier row in `lib.rs` that says **truly-unrepresentable** is a claim
//! that some program does *not* build. A suite made only of green tests
//! grades that claim on the easy half; these cases are the red run against a
//! deliberately-broken input that §II.3's standing rule requires.
//!
//! # Why every case carries an `EXPECT:` marker
//!
//! `trybuild` fails a case only when the file **compiles**. It says nothing
//! about *which* error fired, so several claims can share one file while
//! only some of them are actually proven — and that is not hypothetical:
//! two `E0451` private-field claims in this corpus were silently suppressed
//! by rustc because an earlier error in the same body had already fired. The
//! files still failed to compile. The corpus still went green. Those two
//! guarantees were being asserted, not proven — the vacuous-guard shape,
//! inside the vacuous-guard primitive's own tests.
//!
//! So each `tests/ui/*.rs` declares one `// EXPECT: <substring>` line per
//! claim, and [`every_declared_claim_is_present_in_its_recorded_stderr`]
//! checks each substring against the recorded output. A claim that stops
//! firing now goes red instead of going quiet.
//!
//! # Regenerating
//!
//! ```text
//! TRYBUILD=overwrite cargo test -p gen-verdict --test compile_fail
//! ```
//!
//! Read the diff before accepting it: an error code that *changes* is a
//! toolchain detail, but an error that *disappears* is this crate's
//! guarantee disappearing.
//!
//! # And read it for PATHS, which is a different failure
//!
//! `trybuild` rewrites locations outside the test case to `$RUST`, `$CARGO`,
//! `$WORKSPACE` and `$DIR` — but only when it recognises the shape. It knows
//! `/rustlib/src/rust/library/` and `/rustc/<40-hex>/library/`; it does not
//! know a rustc whose debug paths point into the sandbox it was BUILT in, and
//! that is the rustc a `nix develop` shell hands you. Bless on such a machine
//! and the recorded expectation becomes, verbatim:
//!
//! ```text
//!   --> /nix/var/nix/builds/nix-36689-2103234046/rustc-1.91.1-src/library/core/src/convert/mod.rs:592:8
//! ```
//!
//! — a directory name that will not exist on the next build of the same
//! derivation, let alone on a runner. That line was committed in `05675d8` and
//! failed every CI run of this crate until 2026-08-22, always as a `mismatch`
//! whose diff is one path, which reads like a toolchain drift and is not.
//! [`no_recorded_stderr_pins_a_machine_local_path`] turns it into a named
//! failure at the moment of blessing instead.

use std::path::{Path, PathBuf};

use gen_verdict::{Subjects, Verdict};

#[test]
fn illegal_states_do_not_compile() {
    trybuild::TestCases::new().compile_fail("tests/ui/*.rs");
}

/// One unmet claim in the compile-fail corpus.
#[derive(Debug, PartialEq, Eq, Hash, Clone)]
struct Unmet {
    case: String,
    expected: String,
}

/// The corpus checker, expressed through the primitive it checks.
///
/// Deliberate dogfood, and not decoration: if `tests/ui/` were ever emptied
/// or the glob ever stopped matching, a hand-rolled `for` loop over the
/// files would find zero problems and report success — the exact defect this
/// crate exists to remove. Routed through [`Verdict`], an empty corpus is
/// [`Verdict::Vacuous`], which is not a pass, and this test says so.
#[test]
fn every_declared_claim_is_present_in_its_recorded_stderr() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/ui");
    let mut cases: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("tests/ui must exist")
        .filter_map(|e| {
            let p = e.ok()?.path();
            (p.extension()? == "rs").then_some(p)
        })
        .collect();
    cases.sort();

    let mut findings: Vec<Unmet> = Vec::new();
    for case in &cases {
        let name = case
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_owned();
        let source = std::fs::read_to_string(case).expect("case is readable");
        let expectations: Vec<&str> = source
            .lines()
            .filter_map(|l| l.trim().strip_prefix("// EXPECT: "))
            .collect();

        if expectations.is_empty() {
            findings.push(Unmet {
                case: name.clone(),
                expected: "at least one `// EXPECT: <substring>` marker".to_owned(),
            });
            continue;
        }

        let recorded = std::fs::read_to_string(case.with_extension("stderr")).unwrap_or_default();
        for expected in expectations {
            if !recorded.contains(expected) {
                findings.push(Unmet {
                    case: name.clone(),
                    expected: expected.to_owned(),
                });
            }
        }
    }

    let verdict: Verdict<PathBuf, Unmet> = Verdict::judge(Subjects::scope(cases), findings);

    match &verdict {
        Verdict::Held { subjects, .. } => {
            assert!(
                subjects.count().get() >= 9,
                "the corpus shrank to {} case(s); a deleted compile-fail case is a \
                 deleted guarantee",
                subjects.count(),
            );
        }
        Verdict::Falsified { findings, .. } => {
            let detail: Vec<String> = findings
                .iter()
                .map(|f| {
                    let mut s = String::from("  - ");
                    s.push_str(&f.case);
                    s.push_str(": expected stderr to contain \"");
                    s.push_str(&f.expected);
                    s.push('"');
                    s
                })
                .collect();
            panic!(
                "{} declared compile-fail claim(s) are not present in the recorded \
                 stderr — a suppressed error is an unproven guarantee:\n{}",
                findings.count(),
                detail.join("\n"),
            );
        }
        Verdict::Vacuous => panic!(
            "the compile-fail corpus is EMPTY. A corpus that checks nothing \
             reports no problems; that is not a pass.",
        ),
        Verdict::Unreached => unreachable!("judge never yields Unreached"),
    }
}

/// One recorded line whose path only exists on the machine that blessed it.
#[derive(Debug, PartialEq, Eq, Hash, Clone)]
struct Unportable {
    case: String,
    line: String,
}

/// A blessed `.stderr` must not pin an absolute path.
///
/// `trybuild` normalizes every location it recognises, so a portable recording
/// carries only workspace-relative paths and `$RUST` / `$CARGO` / `$WORKSPACE`
/// / `$DIR` placeholders. An absolute one is not a stricter expectation — it is
/// an expectation no other machine can meet, and it fails as `mismatch`, the
/// same word trybuild uses when a guarantee genuinely changed. The two are
/// indistinguishable in the diff; only this test tells them apart.
///
/// Routed through [`Verdict`] for the same reason its sibling is: a scan that
/// finds no files finds no problems, and that is not a pass.
#[test]
fn no_recorded_stderr_pins_a_machine_local_path() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/ui");
    let mut recordings: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("tests/ui must exist")
        .filter_map(|e| {
            let p = e.ok()?.path();
            (p.extension()? == "stderr").then_some(p)
        })
        .collect();
    recordings.sort();

    let mut findings: Vec<Unportable> = Vec::new();
    for recording in &recordings {
        let name = recording
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_owned();
        let body = std::fs::read_to_string(recording).expect("recording is readable");
        for line in body.lines() {
            let trimmed = line.trim_start();
            // `--> /…` is the shape trybuild failed to normalize. Everything it
            // DID normalize starts with `$`, and everything inside the crate is
            // workspace-relative.
            let unnormalized_location = trimmed.starts_with("--> /");
            let leaked_store_path = ["/nix/store/", "/nix/var/", "/home/", "/Users/"]
                .iter()
                .any(|p| line.contains(p));
            if unnormalized_location || leaked_store_path {
                findings.push(Unportable {
                    case: name.clone(),
                    line: line.trim().to_owned(),
                });
            }
        }
    }

    let verdict: Verdict<PathBuf, Unportable> =
        Verdict::judge(Subjects::scope(recordings), findings);

    match &verdict {
        Verdict::Held { subjects, .. } => {
            assert!(
                subjects.count().get() >= 9,
                "the recorded corpus shrank to {} file(s); a deleted recording is a \
                 deleted guarantee",
                subjects.count(),
            );
        }
        Verdict::Falsified { findings, .. } => {
            let detail: Vec<String> = findings
                .iter()
                .map(|f| {
                    let mut s = String::from("  - ");
                    s.push_str(&f.case);
                    s.push_str(": ");
                    s.push_str(&f.line);
                    s
                })
                .collect();
            panic!(
                "{} recorded line(s) pin an absolute path. trybuild could not \
                 normalize the location, so this expectation is met only on the \
                 machine that blessed it and reports `mismatch` everywhere else — \
                 the same word it uses for a real regression. Re-bless in an \
                 environment whose rustc reports `$RUST/...`, or hand-normalize \
                 the location:\n{}",
                findings.count(),
                detail.join("\n"),
            );
        }
        Verdict::Vacuous => panic!(
            "tests/ui holds NO recorded stderr. A scan over nothing finds no \
             problems; that is not a pass.",
        ),
        Verdict::Unreached => unreachable!("judge never yields Unreached"),
    }
}
