//! The conformance corpus for the derived-verdict law, run from **outside**
//! the crate.
//!
//! Being an external crate is load-bearing, not incidental: `Held` and
//! `Falsified` are `#[non_exhaustive]`, so every construction attempt and
//! every pattern in this file exercises the downstream tier — the one this
//! crate actually claims to be truly-unrepresentable. An in-crate test would
//! be grading the module on privileges no consumer has.
//!
//! Three tests are the generic reproduction of the fleet's best-in-class
//! instance (`magma-types/src/observation.rs`, tests at :438-612):
//!
//! | magma | here |
//! |---|---|
//! | `an_empty_probe_set_is_vacuous_not_complete` | [`an_empty_subject_set_is_vacuous_never_held`] |
//! | `in_sync_carries_its_subject_set_witness` | [`a_held_verdict_carries_its_subject_set_witness`] |
//! | `a_forged_complete_record_is_rejected_at_the_parse_boundary` | [`a_forged_held_record_is_rejected_at_the_parse_boundary`] |

use gen_verdict::{Domain, NonEmpty, Subjects, Verdict};

// The finding type is `String` rather than `&'static str` so the wire tests
// can deserialize from a locally-owned buffer — a borrowed finding would tie
// every parsed verdict to the lifetime of the bytes it came from.
type V = Verdict<u8, String>;

fn judge(subjects: Vec<u8>, findings: &[&str]) -> V {
    Verdict::judge(
        Subjects::scope(subjects),
        findings.iter().map(|f| (*f).to_owned()).collect(),
    )
}

// ── the arm matrix — a forcing function, not a checklist ────────────
//
// `arm_of` is an EXHAUSTIVE external match over `Verdict`. Adding a fifth
// arm without adding its row here is a compile error in this file, so the
// matrix cannot silently fall behind the type (★★ CLOSED-LOOP
// MASS-SYNTHESIS rule 1). There is deliberately no `_ =>` arm; a wildcard
// is exactly the mechanism §II.3's subclass D describes rounding an empty
// arm up.

fn arm_of(v: &V) -> &'static str {
    match v {
        Verdict::Held { .. } => "held",
        Verdict::Falsified { .. } => "falsified",
        Verdict::Vacuous => "vacuous",
        Verdict::Unreached => "unreached",
    }
}

struct Row {
    arm: &'static str,
    verdict: fn() -> V,
    claims_a_pass: bool,
    subject_count: usize,
}

fn matrix() -> Vec<Row> {
    vec![
        Row {
            arm: "held",
            verdict: || judge(vec![1, 2, 3], &[]),
            claims_a_pass: true,
            subject_count: 3,
        },
        Row {
            arm: "falsified",
            verdict: || judge(vec![1, 2], &["bad"]),
            claims_a_pass: false,
            subject_count: 2,
        },
        Row {
            arm: "vacuous",
            verdict: || judge(Vec::new(), &[]),
            claims_a_pass: false,
            subject_count: 0,
        },
        Row {
            arm: "unreached",
            verdict: || Verdict::Unreached,
            claims_a_pass: false,
            subject_count: 0,
        },
    ]
}

#[test]
fn every_arm_has_a_matrix_row_and_the_rows_agree_with_the_type() {
    let mut failures = Vec::new();
    for row in matrix() {
        let v = (row.verdict)();
        if arm_of(&v) != row.arm {
            failures.push(row.arm);
            continue;
        }
        if v.kind() != row.arm {
            failures.push(row.arm);
            continue;
        }
        if v.is_held() != row.claims_a_pass {
            failures.push(row.arm);
            continue;
        }
        if v.subject_count() != row.subject_count {
            failures.push(row.arm);
        }
    }
    assert!(
        failures.is_empty(),
        "{} arm(s) disagree with their matrix row: {failures:?}",
        failures.len(),
    );
}

#[test]
fn the_matrix_covers_the_whole_closed_sum() {
    assert_eq!(
        matrix().len(),
        4,
        "the sum is four-way; a row is missing or duplicated",
    );
}

#[test]
fn exactly_one_arm_claims_a_pass() {
    let passes: Vec<_> = matrix()
        .into_iter()
        .filter(|r| ((r.verdict)()).is_held())
        .map(|r| r.arm)
        .collect();
    assert_eq!(passes, vec!["held"]);
}

// ── clause 2 — the witness ──────────────────────────────────────────

/// magma's `an_empty_probe_set_is_vacuous_not_complete`, generic.
#[test]
fn an_empty_subject_set_is_vacuous_never_held() {
    let v = judge(Vec::new(), &[]);
    assert_eq!(
        arm_of(&v),
        "vacuous",
        "a check that examined nothing must never claim a pass",
    );
    assert!(!v.is_held());
    assert!(v.subjects().is_none(), "there is no witness to report");
}

/// magma's `in_sync_carries_its_subject_set_witness`, generic — and one rung
/// past it. magma's `InSync { in_sync: usize }` carries a *count*, so
/// `InSync { in_sync: 0 }` is constructible; here the witness is the
/// examined values themselves and the zero case has no inhabitant.
#[test]
fn a_held_verdict_carries_its_subject_set_witness() {
    let v = judge(vec![10, 20, 30], &[]);
    let Verdict::Held { subjects, .. } = &v else {
        panic!("three clean subjects must be held");
    };
    assert_eq!(subjects.count().get(), 3);
    assert_eq!(
        subjects.iter().copied().collect::<Vec<_>>(),
        vec![10, 20, 30]
    );
    assert_eq!(v.subject_count(), 3);
}

#[test]
fn a_falsified_verdict_carries_both_witnesses() {
    let v = judge(vec![1, 2, 3], &["a", "b"]);
    let Verdict::Falsified {
        subjects, findings, ..
    } = &v
    else {
        panic!("findings must falsify");
    };
    assert_eq!(subjects.count().get(), 3);
    assert_eq!(findings.count().get(), 2);
}

#[test]
fn vacuous_and_unreached_are_distinct_no_claim_arms() {
    let vacuous = judge(Vec::new(), &[]);
    assert_ne!(
        arm_of(&vacuous),
        arm_of(&Verdict::Unreached),
        "\"nothing was in scope\" and \"this never ran\" are different facts",
    );
    assert!(!vacuous.is_held() && !Verdict::<u8, String>::Unreached.is_held());
}

#[test]
fn the_default_verdict_is_the_one_that_claims_nothing() {
    // A verdict field nobody assigned must read as "no claim", never as a
    // pass. This is subclass D's fail-closed default expressed as `Default`.
    let v = V::default();
    assert_eq!(arm_of(&v), "unreached");
    assert!(!v.is_held());
}

// ── clause 1 — derivation ───────────────────────────────────────────

#[test]
fn scoping_returns_a_named_sum_not_an_option() {
    // §III.6: `Option` invites `unwrap_or_default()`, which is the silent
    // round-up. A `Domain` has no `unwrap_or_default` to reach for — see
    // tests/ui/domain_is_not_an_option.rs for the compile-fail proof.
    assert!(matches!(
        Subjects::scope(Vec::<u8>::new()),
        Domain::<u8>::Empty
    ));
    assert!(matches!(Subjects::scope(vec![1_u8]), Domain::Populated(_)));
}

#[test]
fn findings_over_an_empty_domain_cannot_promote_it_to_a_claim() {
    assert_eq!(arm_of(&judge(Vec::new(), &["impossible"])), "vacuous");
}

/// The vacuity mutation. A verdict primitive whose own corpus never mutates
/// a subject set to empty would be the joke that writes itself.
///
/// This is the shape §II.3's subclass A describes and the shape
/// `sui/sui-spec/src/parity.rs:359-362` still ships: `all(...)` over an
/// empty collection is `true`, faithfully `#[must_use]`-enforced, and means
/// nothing. Both forms are run over the same mutated input; only one of
/// them goes red.
#[test]
fn the_naive_all_pass_form_goes_green_on_the_mutation_that_this_type_catches() {
    fn naive_all_pass(subjects: &[u8]) -> bool {
        subjects.iter().all(|s| *s % 2 == 0)
    }

    let populated = vec![2_u8, 4, 6];
    let mutated_to_empty: Vec<u8> = Vec::new();

    // Same answer from the naive form on both inputs — the mutation is
    // invisible.
    assert!(naive_all_pass(&populated));
    assert!(
        naive_all_pass(&mutated_to_empty),
        "this is the defect, reproduced: vacuous truth reads as a pass",
    );

    // The typed form separates them.
    let over_populated = judge(populated.clone(), &[]);
    let over_empty = judge(mutated_to_empty, &[]);
    assert_eq!(arm_of(&over_populated), "held");
    assert_eq!(
        arm_of(&over_empty),
        "vacuous",
        "the mutation to an empty subject set must change the verdict",
    );
    assert_ne!(arm_of(&over_populated), arm_of(&over_empty));
}

// ── the wire border ─────────────────────────────────────────────────

#[test]
fn every_arm_round_trips_through_serde() {
    for row in matrix() {
        let v = (row.verdict)();
        let json = serde_json::to_string(&v).expect("serialize");
        let back: V = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(arm_of(&back), row.arm, "round trip changed the arm: {json}");
        assert_eq!(back.subject_count(), row.subject_count);
    }
}

#[test]
fn vacuous_survives_a_round_trip_as_a_distinct_tag() {
    let json = serde_json::to_string(&judge(Vec::new(), &[])).expect("serialize");
    assert_eq!(
        json, r#"{"verdict":"vacuous"}"#,
        "the empty case must have its own tag on the wire, not a held tag \
         with an empty list",
    );
    let back: V = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(arm_of(&back), "vacuous");
    assert!(!back.is_held());
}

/// magma's `a_forged_complete_record_is_rejected_at_the_parse_boundary`,
/// generic. The record is well-formed JSON of the right shape; only its
/// *claim* is a lie, and the claim is re-derived rather than trusted.
#[test]
fn a_forged_held_record_is_rejected_at_the_parse_boundary() {
    let forged = r#"{"verdict":"held","subjects":[]}"#;
    let err = serde_json::from_str::<V>(forged).expect_err("a pass over nothing must not parse");
    let msg = err.to_string();
    assert!(
        msg.contains("held") && msg.contains("vacuous"),
        "the rejection must name both the claim and the truth: {msg}",
    );
}

#[test]
fn forged_records_are_rejected_rather_than_normalized() {
    // Every one of these is a record whose declared arm contradicts its own
    // witness. None of them may become a live value, and none of them may be
    // silently corrected into a different arm: a corrected record hides that
    // something shipped a claim larger than what it examined.
    for forged in [
        r#"{"verdict":"held","subjects":[]}"#,
        r#"{"verdict":"held","subjects":[1],"findings":["bad"]}"#,
        r#"{"verdict":"falsified","subjects":[1]}"#,
        r#"{"verdict":"falsified","subjects":[],"findings":["bad"]}"#,
        r#"{"verdict":"vacuous","subjects":[1]}"#,
        r#"{"verdict":"unreached","subjects":[1]}"#,
        r#"{"verdict":"unreached","findings":["bad"]}"#,
    ] {
        assert!(
            serde_json::from_str::<V>(forged).is_err(),
            "forged record parsed: {forged}",
        );
    }
}

#[test]
fn an_empty_witness_is_rejected_on_its_own() {
    assert!(serde_json::from_str::<Subjects<u8>>("[]").is_err());
    assert!(serde_json::from_str::<Subjects<u8>>("[1]").is_ok());
}

// ── the permit ──────────────────────────────────────────────────────

#[test]
fn only_a_held_verdict_mints_a_permit() {
    let mut minted = Vec::new();
    for row in matrix() {
        if (row.verdict)().into_permit().is_ok() {
            minted.push(row.arm);
        }
    }
    assert_eq!(minted, vec!["held"]);
}

#[test]
fn the_permit_hands_the_guarded_action_the_examined_set() {
    fn guarded(permit: gen_verdict::Permit<u8>) -> Vec<u8> {
        // The signature is the gate: this function cannot be called without
        // a permit, and a permit has exactly one minter.
        permit.authorize(|examined| examined.into_vec())
    }

    let permit = judge(vec![7, 8], &[]).into_permit().expect("held");
    assert_eq!(permit.count().get(), 2);
    assert_eq!(guarded(permit), vec![7, 8]);
}

#[test]
fn a_refused_permit_returns_the_verdict_with_its_reason_intact() {
    let refused = judge(vec![1], &["bad"])
        .into_permit()
        .expect_err("findings deny the permit");
    assert_eq!(arm_of(&refused), "falsified");
    assert_eq!(refused.findings().map(|f| f.count().get()), Some(1));
}

// ── the witness cannot be emptied after construction ────────────────

#[test]
fn growing_a_witness_keeps_it_non_empty_and_there_is_no_shrinking_door() {
    // `push` is the one mutation that cannot violate the invariant, which
    // is why it is the only one. `pop`/`clear`/`drain`/`DerefMut` are absent
    // — proven in tests/ui/subjects_cannot_be_emptied.rs.
    let mut s = NonEmpty::one(1_u8);
    s.push(2);
    s.push(3);
    assert_eq!(s.count().get(), 3);
}
