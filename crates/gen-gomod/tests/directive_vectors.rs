//! The Rust half of the SHARED vector table.
//!
//! Reads `substrate/lib/build/go/directive-vectors.json` — the SAME file
//! substrate's `tests/directive-test.nix` reads. One byte edited there must
//! turn BOTH suites red. If only one goes red, a second copy of the ordering
//! rule exists somewhere, which is the defect the shared table prevents.
//!
//! # It FAILS rather than skips
//!
//! A cross-repo test that silently skips when it cannot find its input is
//! worse than no test: it reports green having verified nothing, which is the
//! `--if-present` vacuity this fleet has already been bitten by. If the table
//! is not found, this fails naming the variable and every path it tried.

use gen_gomod::directive::{Verdict, classify};
use std::path::PathBuf;

fn table_path() -> PathBuf {
    if let Ok(p) = std::env::var("GO_DIRECTIVE_VECTORS") {
        return PathBuf::from(p);
    }
    // The sibling checkout, the layout every pleme-io dev machine has.
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let sibling = here
        .ancestors()
        .nth(3)
        .map(|root| root.join("substrate/lib/build/go/directive-vectors.json"));
    match sibling {
        Some(p) if p.exists() => p,
        other => panic!(
            "directive vectors not found. This test binds gen's predicate to \
             substrate's committed table and REFUSES to skip.\n  \
             set GO_DIRECTIVE_VECTORS=<path to substrate/lib/build/go/directive-vectors.json>\n  \
             tried: {:?}",
            other
        ),
    }
}

#[derive(serde::Deserialize)]
struct Table {
    vectors: Vec<Vector>,
}

#[derive(serde::Deserialize)]
struct Vector {
    directive: String,
    #[serde(rename = "fleetGo")]
    fleet_go: String,
    verdict: String,
}

#[test]
fn every_vector_agrees_with_the_rust_predicate() {
    let raw = std::fs::read_to_string(table_path()).expect("read the shared table");
    let t: Table = serde_json::from_str(&raw).expect("parse the shared table");
    assert!(!t.vectors.is_empty(), "the shared table is empty");

    let mut failures = Vec::new();
    for v in &t.vectors {
        let got = classify(&v.directive, &v.fleet_go).as_str();
        if got != v.verdict {
            failures.push(format!(
                "  directive {:?} vs fleet {:?}: expected {}, got {}",
                v.directive, v.fleet_go, v.verdict, got
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "gen's predicate disagrees with the shared table on {} vector(s):\n{}\n\n\
         Substrate's directive-test.nix reads this SAME file. If only one side is \
         red, a second copy of the ordering rule exists.",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn every_verdict_arm_has_a_vector() {
    // Anti-vacuity, mirroring substrate's identical assertion: a table-driven
    // suite is exactly the shape that quietly empties out.
    let raw = std::fs::read_to_string(table_path()).expect("read the shared table");
    let t: Table = serde_json::from_str(&raw).expect("parse");
    let covered: Vec<&str> = t.vectors.iter().map(|v| v.verdict.as_str()).collect();
    let missing: Vec<&str> = Verdict::ALL
        .iter()
        .map(|a| a.as_str())
        .filter(|a| !covered.contains(a))
        .collect();
    assert!(
        missing.is_empty(),
        "verdict arm(s) with NO vector: {:?} — an arm with no vector is a branch nothing exercises",
        missing
    );
}

#[test]
fn the_ordering_rule_itself() {
    // The fact the whole predicate exists for, asserted directly so it cannot
    // be lost to a table edit: cmd/go sorts `1.N` BELOW `1.N.0`.
    assert_eq!(classify("1.25", "1.25.0"), Verdict::BareMinor);
    assert_eq!(classify("1.25.0", "1.25.0"), Verdict::Satisfiable);
}
