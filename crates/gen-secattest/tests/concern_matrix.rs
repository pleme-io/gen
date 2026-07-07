//! The security-CONCERN verification MATRIX — CLOSED-LOOP MASS-SYNTHESIS
//! rule 1 applied to the layered enjulho ship.
//!
//! ONE matrix with ONE row per CONCERN, each asserting the concern is
//! **present** (a named best-in-class OSS engine), **control-mapped** (≥1
//! NIST 800-53 Rev 5 control), and correctly declared **fail-closed** (its
//! failure denies the typed run path IFF it maps to a required verdict kind).
//!
//! The two forcing functions the doctrine names:
//!   - `matrix_covers_every_concern` — the matrix rows ⇄ the concern catalog
//!     are the SAME set; a new concern cannot ship a catalog slot without a
//!     matrix row (and vice versa).
//!   - `no_verdict_slot_rounded_up` — no concern rounds its maturity or its
//!     fail-closed claim UP past what the typed law actually earns.
//!
//! Failures AGGREGATE before the assert — one run reports every broken row.

use gen_secattest::concern::{
    concern_slot, maturity_histogram, Concern, ConcernSlot, ShipMaturity, CONCERN_CATALOG,
};
use gen_secattest::fixture;
use gen_secattest::invariant::Attestable;
use gen_secattest::verdict::{CveDbEpoch, VerifyContext};

/// One row of the concern matrix: a concern + its expected fail-closed
/// posture (the catalog cross-checks the engine/controls/maturity).
struct ConcernRow {
    concern: Concern,
    expected_fail_closed: bool,
}

/// The matrix. ONE row per concern in the catalog. Adding a concern slot
/// without a row here fails `matrix_covers_every_concern`.
const MATRIX: &[ConcernRow] = &[
    ConcernRow { concern: Concern::Signing, expected_fail_closed: true },
    ConcernRow { concern: Concern::Sbom, expected_fail_closed: true },
    ConcernRow { concern: Concern::CveVex, expected_fail_closed: true },
    ConcernRow { concern: Concern::ProvenanceVsa, expected_fail_closed: true },
    ConcernRow { concern: Concern::Transparency, expected_fail_closed: false },
    ConcernRow { concern: Concern::Guac, expected_fail_closed: false },
    ConcernRow { concern: Concern::Scorecard, expected_fail_closed: false },
    ConcernRow { concern: Concern::Admission, expected_fail_closed: false },
];

#[test]
fn matrix_covers_every_concern() {
    // The forcing function: matrix rows and catalog slots are the SAME set.
    let mut catalog_concerns: Vec<Concern> = CONCERN_CATALOG.iter().map(|s| s.concern).collect();
    let mut matrix_concerns: Vec<Concern> = MATRIX.iter().map(|r| r.concern).collect();
    catalog_concerns.sort_unstable();
    matrix_concerns.sort_unstable();
    assert_eq!(
        catalog_concerns, matrix_concerns,
        "concern matrix ⇄ catalog drift: every concern needs exactly one matrix row"
    );
    assert!(MATRIX.len() >= 8, "matrix regressed below the eight known security concerns");
}

#[test]
fn every_concern_is_present_control_mapped_and_correctly_gated() {
    // The per-row law: present (engine named) + control-mapped (≥1 800-53) +
    // fail-closed declared exactly as the typed law earns it. Aggregates.
    let mut failures: Vec<String> = Vec::new();
    for row in MATRIX {
        let Some(slot): Option<&ConcernSlot> = concern_slot(row.concern) else {
            failures.push(format!("{:?}: no catalog slot", row.concern));
            continue;
        };
        // present
        if slot.engine.is_empty() {
            failures.push(format!("{:?}: no best-in-class OSS engine named (not present)", row.concern));
        }
        // control-mapped
        if slot.controls.is_empty() {
            failures.push(format!("{:?}: no 800-53 control mapping (not control-mapped)", row.concern));
        }
        // fail-closed declared correctly
        if slot.fail_closed != row.expected_fail_closed {
            failures.push(format!(
                "{:?}: catalog fail_closed={} but matrix expects {}",
                row.concern, slot.fail_closed, row.expected_fail_closed
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} concern row(s) failed present+control-mapped+fail-closed:\n  - {}",
        failures.len(),
        failures.join("\n  - ")
    );
}

#[test]
fn fail_closed_concerns_actually_deny_the_run_path() {
    // Teeth: for every concern the matrix marks fail-closed, dropping its
    // produced verdict kind from an otherwise-complete verdict must DENY the
    // run token (S1 fail-closed). This binds the CONCERN catalog to the
    // running `verify()` law — a concern cannot merely *claim* fail-closed.
    let addr = fixture::addr("payload");
    let ctx = VerifyContext::strict(CveDbEpoch(100));
    let mut failures: Vec<String> = Vec::new();
    for row in MATRIX.iter().filter(|r| r.expected_fail_closed) {
        let kind = row
            .concern
            .required_verdict_kind()
            .expect("a fail-closed concern maps to a required verdict kind");
        let art = fixture::FixtureArtifact {
            addr: addr.clone(),
            verdict: fixture::incomplete_partial(&addr, CveDbEpoch(100), kind),
        };
        if art.admit(&ctx).is_ok() {
            failures.push(format!(
                "{:?}: dropping its verdict ({kind:?}) STILL admitted — not fail-closed",
                row.concern
            ));
        }
    }
    assert!(failures.is_empty(), "fail-closed concerns that did not deny:\n  - {}", failures.join("\n  - "));
}

#[test]
fn no_verdict_slot_rounded_up() {
    // The honesty gate. Two rounds-up are made impossible:
    //   (1) MATURITY: the ledger is the exact honest snapshot; nothing
    //       silently promotes Design → Wired → Shipped, and NOTHING claims
    //       Shipped (the honest ceiling — no live-enforced + e2e-proven
    //       concern exists yet).
    //   (2) FAIL-CLOSED: a concern claims fail-closed IFF it produces a
    //       required verdict kind — an advisory/evidence/beside concern
    //       (transparency/guac/scorecard/admission) cannot round its
    //       posture up to "denies the run path."
    let (design, wired, shipped) = maturity_histogram();
    assert_eq!(design + wired + shipped, CONCERN_CATALOG.len(), "histogram must partition the catalog");
    assert_eq!(shipped, 0, "no concern may claim Shipped — the honest ceiling (no e2e-proven enforcement yet)");
    assert_eq!(
        (design, wired, shipped),
        (3, 5, 0),
        "concern maturity ledger drifted — promote deliberately, never round up"
    );

    let mut rounded: Vec<String> = Vec::new();
    for slot in CONCERN_CATALOG {
        let produces_required = slot.concern.required_verdict_kind().is_some();
        if slot.fail_closed && !produces_required {
            rounded.push(format!(
                "{:?}: claims fail_closed but produces no required verdict kind (rounded up)",
                slot.concern
            ));
        }
        // A Shipped maturity would require the concern be live-enforced AND
        // e2e-proven; none is. Belt-and-braces with the histogram assert above.
        if slot.maturity == ShipMaturity::Shipped {
            rounded.push(format!("{:?}: maturity=Shipped is not earned yet", slot.concern));
        }
    }
    assert!(rounded.is_empty(), "rounded-up concern(s):\n  - {}", rounded.join("\n  - "));
}
