//! The verdict: one classifier, four arms, a witness on every arm that
//! claims anything, and a wire border that re-derives.

use serde::{Deserialize, Serialize};

use crate::subjects::{Domain, Findings, NonEmpty, Subjects};

/// What a check is entitled to say about the subject set it examined.
///
/// # Clause 1 — derivation
///
/// [`Verdict::judge`] is the sole classifier. The two arms that carry a
/// claim are `#[non_exhaustive]`, so **no code outside this crate can name
/// them** — a struct-expression is a compile error (`E0639`), not a
/// discouraged pattern. Downstream, `judge` is not the recommended ingress;
/// it is the only one.
///
/// This is one rung past the reference implementation. magma's
/// `DriftVerdict::InSync { in_sync: usize }` is a plain public variant, so
/// `InSync { in_sync: 0 }` — a pass over nothing — is constructible by any
/// caller; magma handles it with a doc comment naming the field "the
/// subject-set witness" plus an upstream `Coverage` gate. That is correct
/// for magma, which always has a `Coverage`. It is not enough for a fleet
/// primitive whose callers have none.
///
/// # Clause 2 — witness
///
/// [`Verdict::Held`] cannot be *named* without a [`Subjects<S>`], and a
/// `Subjects<S>` cannot be obtained without having held at least one
/// subject. A pass over an empty set therefore has no expressible form:
/// there is no empty `Subjects` to put in the field, and no way to write the
/// field's absence.
///
/// [`Verdict::Vacuous`] and [`Verdict::Unreached`] are **distinct arms**,
/// never folded into a pass and never folded into each other. "Nothing was
/// in scope" and "this never ran" are different facts, and a consumer that
/// wants to treat them alike must say so in its own `match`.
///
/// # What emptiness *means* is the caller's decision
///
/// This type makes emptiness sayable; it does not decide what it says.
/// magma treats a vacuous refresh as supporting an in-sync claim (an empty
/// world cannot drift). `skill-lint` treats a zero-skill run as an error (a
/// linter that validated nothing validated nothing). Both are right, and
/// neither is expressible while emptiness is silently a pass.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    gen_macros::Discriminant,
    gen_macros::IsVariant,
)]
#[discriminant(method = "kind", case = "snake", also_display)]
#[serde(
    tag = "verdict",
    rename_all = "snake_case",
    try_from = "VerdictWire<S, F>"
)]
pub enum Verdict<S, F> {
    /// The claim holds over a **non-empty** examined subject set.
    ///
    /// `subjects` is the witness. It is not a count, and not a count that
    /// happens to be non-zero: it is the examined values themselves, so the
    /// count and the claim share one origin.
    #[non_exhaustive]
    Held { subjects: Subjects<S> },
    /// The claim fails over a non-empty examined subject set, with at least
    /// one finding. A falsified verdict with zero findings would be a held
    /// verdict wearing the wrong tag, so `findings` is non-empty by the same
    /// construction as `subjects`.
    #[non_exhaustive]
    Falsified {
        subjects: Subjects<S>,
        findings: Findings<F>,
    },
    /// The check ran; **nothing was in scope**. Not a pass. The distinct arm
    /// §II.4's clause 2 requires, and §II.3's subclass A in one word.
    Vacuous,
    /// The check **never ran**. No claim of any kind is available.
    ///
    /// The honest [`Default`], so a verdict field that was never assigned
    /// reads as "no claim" rather than as a pass. `Unreached` is
    /// constructible by anyone — a caller legitimately needs to record the
    /// absence of a run — and constructing it asserts nothing.
    Unreached,
}

impl<S, F> Default for Verdict<S, F> {
    /// [`Verdict::Unreached`] — the only arm that claims nothing.
    fn default() -> Self {
        Self::Unreached
    }
}

impl<S, F> Verdict<S, F> {
    /// **The one classifier.** Total over its inputs; no arm is chosen
    /// anywhere else.
    ///
    /// The ladder, and why each rung is where it is:
    ///
    /// 1. an empty domain ⇒ [`Verdict::Vacuous`] — checked first, so no
    ///    finding count and no caller intent can promote an empty scope to a
    ///    claim;
    /// 2. subjects, no findings ⇒ [`Verdict::Held`];
    /// 3. subjects and findings ⇒ [`Verdict::Falsified`].
    ///
    /// [`Verdict::Unreached`] is deliberately unreachable from `judge`:
    /// `judge` is called *by* a check that ran, so a run cannot classify
    /// itself as never having run.
    ///
    /// # Findings over an empty domain
    ///
    /// `judge(Domain::Empty, findings)` is `Vacuous`, and the findings are
    /// dropped. That combination is a caller defect — a finding is a
    /// statement *about a subject*, so findings over a set that was never
    /// examined are not findings. `judge` stays total rather than
    /// panicking; the behaviour is pinned by test.
    pub fn judge(domain: Domain<S>, findings: Vec<F>) -> Self {
        // Every arm is named. There is deliberately no `_` anywhere in this
        // crate's matches, including in a tuple position: a wildcard is the
        // mechanism §II.3's subclass D describes for rounding a new case up
        // into an existing one, and a classifier is the last place to leave
        // that door open.
        match (domain, NonEmpty::scope(findings)) {
            (Domain::Empty, Domain::Empty | Domain::Populated(_)) => Self::Vacuous,
            (Domain::Populated(subjects), Domain::Empty) => Self::Held { subjects },
            (Domain::Populated(subjects), Domain::Populated(findings)) => {
                Self::Falsified { subjects, findings }
            }
        }
    }

    /// The examined subject set, when there was one.
    ///
    /// `None` for [`Verdict::Vacuous`] and [`Verdict::Unreached`] — the two
    /// arms that examined nothing. This is a *reporting* accessor and is
    /// safe as an `Option` precisely because there is no default witness to
    /// fall back to: `unwrap_or_default()` does not typecheck on
    /// `Option<&Subjects<S>>`.
    pub fn subjects(&self) -> Option<&Subjects<S>> {
        match self {
            Self::Held { subjects, .. } | Self::Falsified { subjects, .. } => Some(subjects),
            Self::Vacuous | Self::Unreached => None,
        }
    }

    /// The findings, when the verdict was falsified.
    pub fn findings(&self) -> Option<&Findings<F>> {
        match self {
            Self::Falsified { findings, .. } => Some(findings),
            Self::Held { .. } | Self::Vacuous | Self::Unreached => None,
        }
    }

    /// How many subjects the verdict is about — `0` for the two no-claim
    /// arms, which is the honest number and says so.
    #[must_use]
    pub fn subject_count(&self) -> usize {
        self.subjects().map_or(0, |s| s.count().get())
    }

    /// Consume the verdict for a permit.
    ///
    /// The **sole minter** of [`Permit`](crate::Permit). `Err` returns the
    /// verdict itself rather than a bare unit, so a refusal keeps its reason
    /// and its witness.
    ///
    /// A `Result` rather than an `Option` on purpose: `Option::unwrap_or_default`
    /// would need `Permit<S>: Default`, which does not exist and never will.
    ///
    /// # Errors
    ///
    /// Returns the verdict unchanged whenever it is not [`Verdict::Held`].
    pub fn into_permit(self) -> Result<crate::Permit<S>, Self> {
        match self {
            Self::Held { subjects } => Ok(crate::Permit::mint(subjects)),
            // Named rather than `refused =>`: a binding catch-all would
            // silently route a future fifth arm into "refused", which is the
            // safe direction but not a decision anybody made.
            refused @ (Self::Falsified { .. } | Self::Vacuous | Self::Unreached) => Err(refused),
        }
    }
}

// ── the wire border: re-derive, and reject rather than normalize ─────

/// The deserialization shape. Every arm carries both sequences so that a
/// record which *declares* one arm while *carrying* the shape of another is
/// visible to the re-derivation rather than silently discarded as an unknown
/// field.
#[derive(Deserialize, gen_macros::Discriminant)]
#[discriminant(method = "kind", case = "snake")]
#[serde(tag = "verdict", rename_all = "snake_case")]
// Stated explicitly because serde's inference sees `#[serde(default)]` on a
// `Vec<S>` field and demands `S: Default`, which no consumer should have to
// satisfy to deserialize a verdict about `S`. The default we want is
// `Vec::new()`, which needs nothing of `S`.
#[serde(bound(deserialize = "S: Deserialize<'de>, F: Deserialize<'de>"))]
enum VerdictWire<S, F> {
    Held {
        #[serde(default)]
        subjects: Vec<S>,
        #[serde(default)]
        findings: Vec<F>,
    },
    Falsified {
        #[serde(default)]
        subjects: Vec<S>,
        #[serde(default)]
        findings: Vec<F>,
    },
    Vacuous {
        #[serde(default)]
        subjects: Vec<S>,
        #[serde(default)]
        findings: Vec<F>,
    },
    Unreached {
        #[serde(default)]
        subjects: Vec<S>,
        #[serde(default)]
        findings: Vec<F>,
    },
}

/// Failure modes of the verdict parse boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
pub enum VerdictError {
    /// A serialized verdict declared an arm its own witness does not derive
    /// — a forged or corrupted record.
    ///
    /// Rejected rather than silently corrected. Correcting it would hide
    /// that something shipped a record claiming more than it examined, which
    /// is the exact event the type exists to surface.
    #[error(
        "verdict declares `{declared}` but its own witness derives `{derived}` \
         ({subjects} subject(s), {findings} finding(s))"
    )]
    DeclaredVerdictContradictsWitness {
        /// The arm the record claimed.
        declared: &'static str,
        /// The arm `judge` derives from the record's own witness.
        derived: &'static str,
        /// Subjects carried by the record.
        subjects: usize,
        /// Findings carried by the record.
        findings: usize,
    },
}

impl<S, F> TryFrom<VerdictWire<S, F>> for Verdict<S, F> {
    type Error = VerdictError;

    fn try_from(wire: VerdictWire<S, F>) -> Result<Self, Self::Error> {
        let declared = wire.kind();
        let (subjects, findings) = match wire {
            VerdictWire::Held {
                subjects, findings, ..
            }
            | VerdictWire::Falsified {
                subjects, findings, ..
            }
            | VerdictWire::Vacuous {
                subjects, findings, ..
            }
            | VerdictWire::Unreached {
                subjects, findings, ..
            } => (subjects, findings),
        };
        let (n_subjects, n_findings) = (subjects.len(), findings.len());

        // `Unreached` is the one arm that is a statement about the ABSENCE
        // of a run rather than a classification of one, so it is accepted on
        // its own terms — and only on its own terms. A record claiming
        // `unreached` while carrying a witness is exactly as forged as one
        // claiming `held` over nothing. (magma reaches the same conclusion
        // for `Coverage::Unrefreshed`, from the same reasoning.)
        if declared == "unreached" {
            if n_subjects == 0 && n_findings == 0 {
                return Ok(Self::Unreached);
            }
            return Err(VerdictError::DeclaredVerdictContradictsWitness {
                declared,
                derived: Self::judge(NonEmpty::scope(subjects), findings).kind(),
                subjects: n_subjects,
                findings: n_findings,
            });
        }

        let derived = Self::judge(NonEmpty::scope(subjects), findings);
        if derived.kind() == declared {
            Ok(derived)
        } else {
            Err(VerdictError::DeclaredVerdictContradictsWitness {
                declared,
                derived: derived.kind(),
                subjects: n_subjects,
                findings: n_findings,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn judged(subjects: Vec<u8>, findings: Vec<&'static str>) -> Verdict<u8, &'static str> {
        Verdict::judge(NonEmpty::scope(subjects), findings)
    }

    #[test]
    fn an_empty_subject_set_is_vacuous_not_held() {
        let v = judged(Vec::new(), Vec::new());
        assert_eq!(v, Verdict::Vacuous, "a check over nothing is not a pass");
        assert!(!v.is_held());
    }

    #[test]
    fn held_carries_its_subject_set_witness() {
        let v = judged(vec![1, 2, 3], Vec::new());
        assert!(v.is_held());
        assert_eq!(v.subject_count(), 3);
        assert_eq!(
            v.subjects().map(|s| s.iter().copied().collect::<Vec<_>>()),
            Some(vec![1, 2, 3]),
        );
    }

    #[test]
    fn findings_make_the_verdict_falsified_and_travel_with_it() {
        let v = judged(vec![1, 2], vec!["bad"]);
        assert!(v.is_falsified());
        assert_eq!(v.subject_count(), 2);
        assert_eq!(v.findings().map(|f| f.count().get()), Some(1));
    }

    #[test]
    fn findings_over_an_empty_domain_are_still_vacuous() {
        // A finding is a statement about a subject. Findings over a set that
        // was never examined cannot promote the empty scope to a claim.
        assert_eq!(judged(Vec::new(), vec!["impossible"]), Verdict::Vacuous);
    }

    #[test]
    fn the_default_verdict_claims_nothing() {
        let v = Verdict::<u8, &str>::default();
        assert_eq!(v, Verdict::Unreached);
        assert!(!v.is_held());
        assert_eq!(v.subject_count(), 0);
    }

    #[test]
    fn vacuous_and_unreached_are_distinct_arms() {
        assert_ne!(judged(Vec::new(), Vec::new()), Verdict::Unreached);
        assert_eq!(judged(Vec::new(), Vec::new()).kind(), "vacuous");
        assert_eq!(Verdict::<u8, &str>::Unreached.kind(), "unreached");
    }

    // ── the wire border ──────────────────────────────────────────

    #[test]
    fn every_arm_round_trips_through_serde() {
        for v in [
            judged(vec![1, 2, 3], Vec::new()),
            judged(vec![1, 2], vec!["bad", "worse"]),
            judged(Vec::new(), Vec::new()),
            Verdict::Unreached,
        ] {
            let json = serde_json::to_string(&v).expect("serialize");
            let back: Verdict<u8, &str> = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(v, back, "round trip must preserve the arm: {json}");
        }
    }

    #[test]
    fn vacuous_survives_a_round_trip_as_a_distinct_tag() {
        let json = serde_json::to_string(&judged(Vec::new(), Vec::new())).expect("serialize");
        assert_eq!(json, r#"{"verdict":"vacuous"}"#);
        let back: Verdict<u8, &str> = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, Verdict::Vacuous);
        assert!(
            !back.is_held(),
            "a vacuous record must never read as a pass"
        );
    }

    #[test]
    fn a_forged_held_over_an_empty_set_is_rejected_at_the_parse_boundary() {
        let forged = r#"{"verdict":"held","subjects":[]}"#;
        let err = serde_json::from_str::<Verdict<u8, &str>>(forged).expect_err("must reject");
        let msg = err.to_string();
        assert!(
            msg.contains("held") && msg.contains("vacuous"),
            "the rejection must name both the claim and the truth: {msg}",
        );
    }

    #[test]
    fn a_forged_held_carrying_findings_is_rejected() {
        let forged = r#"{"verdict":"held","subjects":[1],"findings":["bad"]}"#;
        let err = serde_json::from_str::<Verdict<u8, &str>>(forged).expect_err("must reject");
        assert!(err.to_string().contains("falsified"), "{err}");
    }

    #[test]
    fn a_forged_unreached_carrying_a_witness_is_rejected() {
        let forged = r#"{"verdict":"unreached","subjects":[1,2]}"#;
        assert!(serde_json::from_str::<Verdict<u8, &str>>(forged).is_err());
    }

    #[test]
    fn a_forged_vacuous_carrying_a_witness_is_rejected() {
        let forged = r#"{"verdict":"vacuous","subjects":[1]}"#;
        let err = serde_json::from_str::<Verdict<u8, &str>>(forged).expect_err("must reject");
        assert!(err.to_string().contains("held"), "{err}");
    }

    #[test]
    fn a_forged_falsified_without_findings_is_rejected() {
        let forged = r#"{"verdict":"falsified","subjects":[1]}"#;
        let err = serde_json::from_str::<Verdict<u8, &str>>(forged).expect_err("must reject");
        assert!(err.to_string().contains("held"), "{err}");
    }
}
