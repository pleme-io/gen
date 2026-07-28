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
