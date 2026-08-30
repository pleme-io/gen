//! The SecAttest verification MATRIX — CLOSED-LOOP MASS-SYNTHESIS rule 1.
//!
//! ONE matrix with ONE row per VERDICT KIND, exercising each against the
//! fail-closed law:
//!
//!   > every REQUIRED slot is load-bearing (omitting it denies the run
//!   > token); every NON-required refinement is optional (its absence does
//!   > not deny); a complete-verified-fresh verdict admits; and every clause
//!   > failure mode (tamper / unverified / stale / exploitable-over-budget)
//!   > is fail-closed.
//!
//! The matrix FAILS THE BUILD when:
//!   - a new catalogued kind lands without a matrix row, or a row names a
//!     kind absent from the catalog (`matrix_covers_every_catalogued_kind` —
//!     the forcing function);
//!   - a REQUIRED slot's omission does NOT deny admission
//!     (`every_required_slot_is_load_bearing`);
//!   - a clean verdict is denied, or a broken one is admitted
//!     (`the_law_admits_clean_and_denies_broken`).
//!
//! Failures AGGREGATE before the assert — one run reports every broken row.

use gen_secattest::catalog::CATALOG;
use gen_secattest::fixture::{self, FixtureArtifact};
use gen_secattest::invariant::Attestable;
use gen_secattest::verdict::{CveDbEpoch, VerdictKind, VerifyContext};
use gen_secattest::{Severities, SlotState, verify};

/// One row of the matrix: a verdict kind + whether it is REQUIRED for
/// completeness (the catalog cross-checks it).
struct MatrixRow {
    kind: VerdictKind,
    expected_required: bool,
}

/// The matrix. ONE row per verdict kind in the catalog. Adding a catalog
/// entry without a row here fails `matrix_covers_every_catalogued_kind`.
const MATRIX: &[MatrixRow] = &[
    MatrixRow {
        kind: VerdictKind::Signature,
        expected_required: true,
    },
    MatrixRow {
        kind: VerdictKind::Sbom,
        expected_required: true,
    },
    MatrixRow {
        kind: VerdictKind::Cve,
        expected_required: true,
    },
    MatrixRow {
        kind: VerdictKind::Provenance,
        expected_required: true,
    },
    MatrixRow {
        kind: VerdictKind::Vex,
        expected_required: false,
    },
    MatrixRow {
        kind: VerdictKind::Scorecard,
        expected_required: false,
    },
    MatrixRow {
        kind: VerdictKind::Vsa,
        expected_required: false,
    },
];

const NOW: CveDbEpoch = CveDbEpoch(100);

#[test]
fn matrix_covers_every_catalogued_kind() {
    // The forcing function (CATALOG REFLECTION + CLOSED-LOOP rule 1): the
    // matrix rows and the catalog entries are the SAME set. A new kind cannot
    // ship a catalog entry without a matrix row, and vice versa.
    let mut catalog_kinds: Vec<VerdictKind> = CATALOG.iter().map(|e| e.kind).collect();
    let mut matrix_kinds: Vec<VerdictKind> = MATRIX.iter().map(|r| r.kind).collect();
    catalog_kinds.sort_unstable();
    matrix_kinds.sort_unstable();
    assert_eq!(
        catalog_kinds, matrix_kinds,
        "SecAttest matrix ⇄ catalog drift: every catalogued kind needs exactly one matrix row"
    );
    assert!(
        MATRIX.len() >= 7,
        "matrix regressed below the seven known verdict kinds"
    );
}

#[test]
fn matrix_required_flags_agree_with_the_catalog() {
    // The matrix is a second witness of the completeness partition — a
    // required-flag edit in one place without the other fails here.
    let mut failures = Vec::new();
    for row in MATRIX {
        if row.expected_required != row.kind.required_for_completeness() {
            failures.push(format!(
                "{:?}: matrix says required={}, kind says {}",
                row.kind,
                row.expected_required,
                row.kind.required_for_completeness()
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "required-flag drift:\n  - {}",
        failures.join("\n  - ")
    );
    let required = MATRIX.iter().filter(|r| r.expected_required).count();
    assert_eq!(required, 4, "exactly four required completeness slots");
}

#[test]
fn every_required_slot_is_load_bearing() {
    // THE core matrix assertion. For each REQUIRED kind, dropping it from an
    // otherwise-complete verdict must DENY the run token (S1 fail-closed).
    // For each NON-required kind, its absence must NOT deny (it is a
    // refinement, not a completeness slot). Failures aggregate.
    let addr = fixture::addr("payload");
    let ctx = VerifyContext::strict(NOW);
    let mut failures: Vec<String> = Vec::new();

    for row in MATRIX {
        let art = FixtureArtifact {
            addr: addr.clone(),
            verdict: fixture::incomplete_partial(&addr, NOW, row.kind),
        };
        let admitted = art.admit(&ctx);
        if row.expected_required {
            // Omitting a required slot must deny, with a MissingSlot naming it.
            match admitted {
                Ok(_) => failures.push(format!(
                    "{:?}: required slot omitted but STILL admitted — not fail-closed",
                    row.kind
                )),
                Err(vs) => {
                    let named = vs.iter().any(|v| matches!(
                        v,
                        gen_secattest::VerdictViolation::MissingSlot { kind, .. } if *kind == row.kind
                    ));
                    if !named {
                        failures.push(format!(
                            "{:?}: denied, but no MissingSlot names it — {vs:?}",
                            row.kind
                        ));
                    }
                }
            }
        } else if admitted.is_err() {
            // A refinement's absence must NOT deny an otherwise-complete verdict.
            failures.push(format!(
                "{:?}: non-required refinement absence WRONGLY denied admission — {:?}",
                row.kind,
                admitted.err()
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "{} matrix row(s) failed the load-bearing law:\n  - {}",
        failures.len(),
        failures.join("\n  - ")
    );
}

#[test]
fn the_law_admits_clean_and_denies_broken() {
    // The behavioral spine: a clean verdict admits; each clause failure mode
    // is fail-closed. Proves the law has teeth (not a vacuous pass).
    let addr = fixture::addr("payload");
    let ctx = VerifyContext::strict(NOW);

    // Clean → admits.
    let clean = fixture::complete_verified(&addr, NOW);
    assert!(
        verify(&clean, &ctx).is_ok(),
        "a complete-verified-fresh verdict must admit"
    );

    // S3 unverified → deny.
    let mut unver = fixture::complete_verified(&addr, NOW);
    unver.provenance.state = SlotState::Unchecked;
    assert!(
        verify(&unver, &ctx).is_err(),
        "an unchecked slot must be fail-closed"
    );

    // S4 tampered subject → deny.
    let mut tampered = fixture::complete_verified(&addr, NOW);
    tampered.signature.subject = fixture::addr("attacker-artifact");
    assert!(
        verify(&tampered, &ctx).is_err(),
        "a replayed foreign subject must be rejected"
    );

    // S5 stale → deny.
    let stale = fixture::complete_verified(&addr, CveDbEpoch(1));
    assert!(
        verify(&stale, &ctx).is_err(),
        "a stale CVE scan must be fail-closed"
    );

    // CVE⊖VEX over budget → deny.
    let mut hot = fixture::complete_verified(&addr, NOW);
    hot.cve.raw = Severities::new(1, 0);
    hot.cve.vex_suppressed = Severities::new(0, 0);
    assert!(
        verify(&hot, &ctx).is_err(),
        "an exploitable critical must be fail-closed"
    );
}

#[test]
fn an_incomplete_verdict_cannot_deserialize_into_a_complete_one() {
    // S1 at the WIRE boundary: a JSON verdict missing a required slot is an
    // Err at deserialize (parse-time-rejected), never a downstream value.
    let addr = fixture::addr("payload");
    let subject = addr.as_str();

    // A partial JSON with ONLY a subject (all slots absent).
    let json = format!("{{\"subject\":\"{subject}\"}}");
    let complete: Result<gen_secattest::CompleteVerdict, _> = serde_json::from_str(&json);
    assert!(
        complete.is_err(),
        "an incomplete verdict must be parse-time-rejected at the CompleteVerdict wire boundary"
    );

    // A partial JSON round-trips into a PartialVerdict just fine (lenient).
    let partial: gen_secattest::PartialVerdict = serde_json::from_str(&json).expect("partial ok");
    assert_eq!(partial.missing().len(), 4, "all four required slots absent");
}

#[test]
fn an_empty_attestation_address_is_rejected_at_the_wire() {
    // S2 inherited: the attestation address IS gen_pdc::ContentAddr, so an
    // empty subject is rejected at the deserialize boundary — no forked seal.
    let json = "{\"subject\":\"\"}";
    let partial: Result<gen_secattest::PartialVerdict, _> = serde_json::from_str(json);
    assert!(
        partial.is_err(),
        "an empty attestation address must be parse-rejected (PDC clause 1)"
    );
}
