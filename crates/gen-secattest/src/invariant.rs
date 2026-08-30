//! The fail-closed SecAttest checker — one law, applied to any artifact's
//! per-hash verdict.
//!
//! This is the runtime half of the invariant. The TYPE-LEVEL clauses are
//! already discharged by construction — S1 (completeness) by
//! [`CompleteVerdict`]'s no-`Option` product shape, S2 (address) by
//! [`AttestationAddr`]'s inherited `ContentAddr` seal, S3 (fail-closed) by
//! [`RunToken`]'s missing constructor. The RUNTIME clauses need this checker:
//! S4 (subject-bound), the verified-before-use state (S3 companion), S5
//! (freshness), the CVE⊖VEX threshold gate, the SBOM/SLSA gates, and the S6
//! dedup witness.
//!
//! `/algorithmic-prowess-seal` — best-fit, no ML:
//! - S4 subject-binding: a typed `ContentAddr` equality per slot.
//! - S5 freshness: an integer epoch/TTL comparison (the C2 external-world
//!   ceiling — a runtime fact, correctly OnlyMitigated, never a type).
//! - CVE⊖VEX gate: `exploitable = raw ⊖ vex`, then `≤ thresholds`.
//! - S6 dedup: a content-address set — `served > computed ⇒ reuse observed`
//!   (a graph-reachability quantifier ⇒ CI forcing-function, PDC `CeilingC1`).
//!
//! **Fail-closed is structural:** [`verify`] returns a [`RunToken`] IFF the
//! violation vector is EMPTY. Any violation ⇒ `Err` ⇒ no token ⇒ no run
//! path. There is no partial-admit, no override, no "warn and continue."

use std::collections::HashSet;

use crate::verdict::{
    AttestationAddr, CompleteVerdict, CveDbEpoch, EpochTtl, PartialVerdict, RunToken, Severities,
    SlotState, SlsaLevel, VerdictKind, VerifyContext,
};

/// A universal SecAttest violation — the law's failure modes, named once.
/// Every one of these denies the run token (fail-closed).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VerdictViolation {
    /// S1: a required slot is absent (surfaced from
    /// [`crate::verdict::IncompleteVerdict::into_violations`]).
    MissingSlot {
        subject: AttestationAddr,
        kind: VerdictKind,
    },
    /// S4: a slot attests a DIFFERENT content hash than the artifact — a
    /// lifted/replayed attestation. `slot = None` is the top-level
    /// artifact-address binding (the `Attestable::admit` pre-check).
    SubjectMismatch {
        slot: Option<VerdictKind>,
        expected: AttestationAddr,
        found: AttestationAddr,
    },
    /// S3/S4: a required slot did not cryptographically verify (`Unchecked`
    /// or `Failed`) — verified-before-use denied.
    SlotUnverified { kind: VerdictKind, state: SlotState },
    /// S5: the CVE scan is stale — it ran more than `ttl` epochs behind the
    /// current epoch, so a newer disclosure may apply to unchanged bytes.
    CveStale {
        scanned_epoch: CveDbEpoch,
        current_epoch: CveDbEpoch,
        ttl: EpochTtl,
    },
    /// The CVE⊖VEX gate: EXPLOITABLE (post-VEX) findings exceed the budget.
    /// Fires on reachable criticals/highs, not raw scan noise (B1).
    CveExploitableOverBudget {
        exploitable: Severities,
        budget: Severities,
    },
    /// The SBOM slot carries an EMPTY inventory (0 components) — not a real
    /// CM-8 SBOM.
    SbomEmpty { subject: AttestationAddr },
    /// The provenance slot's SLSA level is below the required minimum (SR-3/4).
    SlsaBelowMinimum {
        found: SlsaLevel,
        minimum: SlsaLevel,
    },
}

impl VerdictViolation {
    /// A stable kebab-case rule name + optional locus, for a `confirm` typed
    /// report + cse-lint. One vocabulary fleet-wide regardless of which slot
    /// raised the clause.
    #[must_use]
    pub fn locus(&self) -> (&'static str, Option<String>) {
        match self {
            VerdictViolation::MissingSlot { subject, .. } => {
                ("secattest-missing-slot", Some(subject.as_str().to_string()))
            }
            VerdictViolation::SubjectMismatch { found, .. } => (
                "secattest-subject-mismatch",
                Some(found.as_str().to_string()),
            ),
            VerdictViolation::SlotUnverified { kind, .. } => {
                ("secattest-slot-unverified", Some(format!("{kind:?}")))
            }
            VerdictViolation::CveStale { .. } => ("secattest-cve-stale", None),
            VerdictViolation::CveExploitableOverBudget { .. } => {
                ("secattest-cve-exploitable-over-budget", None)
            }
            VerdictViolation::SbomEmpty { subject } => {
                ("secattest-sbom-empty", Some(subject.as_str().to_string()))
            }
            VerdictViolation::SlsaBelowMinimum { .. } => ("secattest-slsa-below-minimum", None),
        }
    }
}

/// THE fail-closed gate (S3). Runs every runtime clause over a
/// [`CompleteVerdict`] and returns a [`RunToken`] IFF ZERO violations.
///
/// Because the ONLY caller that can construct a `RunToken` is this function
/// (via the crate-private `RunToken::seal`), and this function mints one
/// only on an empty violation vector, an unverified/tampered/stale/
/// exploitable-over-budget artifact has NO expressible run path.
///
/// Violations AGGREGATE — one call reports every failed clause, not just the
/// first (CLOSED-LOOP MASS-SYNTHESIS rule 1: the operator sees the whole gap).
#[must_use]
pub fn verify(v: &CompleteVerdict, ctx: &VerifyContext) -> Result<RunToken, Vec<VerdictViolation>> {
    let mut violations = Vec::new();

    // S4 subject-bound + S3 verified-before-use, per required slot.
    for (kind, subject, state) in v.required_slots() {
        if subject != &v.subject {
            violations.push(VerdictViolation::SubjectMismatch {
                slot: Some(kind),
                expected: v.subject.clone(),
                found: subject.clone(),
            });
        }
        if !state.is_verified() {
            violations.push(VerdictViolation::SlotUnverified {
                kind,
                state: state.clone(),
            });
        }
    }

    // SBOM must be a real inventory (nonzero components).
    if v.sbom.component_count == 0 {
        violations.push(VerdictViolation::SbomEmpty {
            subject: v.subject.clone(),
        });
    }

    // Provenance must meet the minimum SLSA level.
    if v.provenance.slsa_level < ctx.min_slsa_level {
        violations.push(VerdictViolation::SlsaBelowMinimum {
            found: v.provenance.slsa_level,
            minimum: ctx.min_slsa_level,
        });
    }

    // S5 freshness (the CVE time dimension — C2 external-world ceiling).
    if v.cve.is_stale(ctx) {
        violations.push(VerdictViolation::CveStale {
            scanned_epoch: v.cve.scanned_epoch,
            current_epoch: ctx.current_epoch,
            ttl: ctx.ttl,
        });
    }

    // The CVE⊖VEX exploitability gate (B1): the gate reads exploitable, not
    // raw scan noise.
    let exploitable = v.cve.exploitable();
    if !exploitable.within(ctx.cve_thresholds) {
        violations.push(VerdictViolation::CveExploitableOverBudget {
            exploitable,
            budget: ctx.cve_thresholds,
        });
    }

    if violations.is_empty() {
        Ok(RunToken::seal(
            v.subject.clone(),
            ctx.current_epoch,
            VerdictKind::REQUIRED.to_vec(),
        ))
    } else {
        Err(violations)
    }
}

/// The universal security-attestation contract. Any artifact-producing
/// surface — a Nix derivation, an OCI image, a crate, a chart — implements
/// this to declare its attestation address + assembled verdict; the provided
/// [`Attestable::admit`] runs the WHOLE fail-closed law (S1→S5), so a
/// consumer NEVER hand-rolls the gate (the Prime Directive: solve the
/// admission once, in one place).
pub trait Attestable {
    /// The artifact's attestation address = the PDC content hash (S2).
    fn attestation_addr(&self) -> &AttestationAddr;

    /// The assembled (possibly partial) security verdict for this artifact.
    fn verdict(&self) -> PartialVerdict;

    /// Fail-closed admission (provided). Completeness (S1) → address binding
    /// (S2) → the runtime law (S3/S4/S5 via [`verify`]). Returns a run token
    /// or EVERY violation. Never overridden by a consumer.
    ///
    /// # Errors
    /// The aggregated [`VerdictViolation`]s — any non-empty vector denies the
    /// run token (fail-closed).
    fn admit(&self, ctx: &VerifyContext) -> Result<RunToken, Vec<VerdictViolation>> {
        let partial = self.verdict();

        // S2 — the artifact's own address must match the verdict subject.
        let mut pre = Vec::new();
        if partial.subject != *self.attestation_addr() {
            pre.push(VerdictViolation::SubjectMismatch {
                slot: None,
                expected: self.attestation_addr().clone(),
                found: partial.subject.clone(),
            });
        }

        // S1 — completeness parse-boundary. Incompleteness folds into the
        // same violation channel; we still surface the pre-check binding.
        let complete = match partial.into_complete() {
            Ok(c) => c,
            Err(inc) => {
                let mut vs = inc.into_violations();
                vs.append(&mut pre);
                return Err(vs);
            }
        };

        // S3/S4/S5 — the runtime law.
        match verify(&complete, ctx) {
            Ok(token) if pre.is_empty() => Ok(token),
            Ok(_) => Err(pre),
            Err(mut vs) => {
                vs.append(&mut pre);
                Err(vs)
            }
        }
    }
}

/// The verdict-dedup witness (S6 — inherits PDC `CeilingC1`).
///
/// A TIMELESS verdict (signature/SBOM/provenance) is a pure function of the
/// bytes, so it is computed ONCE per content hash and reused by every
/// consumer of that hash. Given the stream of consumer requests for an
/// address, `computed` = distinct addresses and `served` = total requests;
/// `served > computed` means a shared hash's verdict was reused — the
/// build-once-reuse win, quantified. It is a graph-reachability property (no
/// dependent-type proof in Rust) ⇒ a mechanical CI forcing-function, the
/// honest C1 ceiling.
#[derive(Clone, Copy, Debug)]
pub struct DedupOutcome {
    /// Distinct attestation addresses whose timeless verdict was computed.
    pub computed: usize,
    /// Total consumer requests served (from the once-computed verdict).
    pub served: usize,
}

impl DedupOutcome {
    /// Was any timeless verdict reused by >1 consumer (dedup observed)?
    #[must_use]
    pub fn observed_dedup(&self) -> bool {
        self.served > self.computed
    }
}

/// Compute the S6 dedup witness over a stream of consumer requests (each an
/// attestation address a consumer needs the timeless verdict for). Pure,
/// deterministic, no I/O.
pub fn dedup_witness<'a, I>(requests: I) -> DedupOutcome
where
    I: IntoIterator<Item = &'a AttestationAddr>,
{
    let mut seen: HashSet<&str> = HashSet::new();
    let mut served = 0usize;
    for a in requests {
        served += 1;
        seen.insert(a.as_str());
    }
    DedupOutcome {
        computed: seen.len(),
        served,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture;

    #[test]
    fn a_complete_verified_fresh_verdict_mints_a_run_token() {
        let addr = fixture::addr("payload");
        let complete = fixture::complete_verified(&addr, CveDbEpoch(100));
        let ctx = VerifyContext::strict(CveDbEpoch(100));
        let token = verify(&complete, &ctx).expect("a clean verdict must admit");
        assert_eq!(token.addr(), &addr);
        assert_eq!(token.verified_kinds().len(), 4);
    }

    #[test]
    fn an_unverified_slot_denies_the_run_token() {
        // S3 verified-before-use: flip the signature to Unchecked ⇒ no token.
        let addr = fixture::addr("payload");
        let mut complete = fixture::complete_verified(&addr, CveDbEpoch(100));
        complete.signature.state = SlotState::Unchecked;
        let ctx = VerifyContext::strict(CveDbEpoch(100));
        let err = verify(&complete, &ctx).expect_err("an unchecked slot must be fail-closed");
        assert!(err.iter().any(|v| matches!(
            v,
            VerdictViolation::SlotUnverified {
                kind: VerdictKind::Signature,
                ..
            }
        )));
    }

    #[test]
    fn a_lifted_attestation_wrong_subject_is_rejected() {
        // S4 subject-bound: a slot attesting a DIFFERENT hash is a replay.
        let addr = fixture::addr("payload");
        let mut complete = fixture::complete_verified(&addr, CveDbEpoch(100));
        complete.provenance.subject = fixture::addr("some-other-artifact");
        let ctx = VerifyContext::strict(CveDbEpoch(100));
        let err = verify(&complete, &ctx).expect_err("a foreign subject must be rejected");
        assert!(err.iter().any(|v| matches!(
            v,
            VerdictViolation::SubjectMismatch {
                slot: Some(VerdictKind::Provenance),
                ..
            }
        )));
    }

    #[test]
    fn a_stale_cve_scan_denies_the_run_token() {
        // S5 freshness: scanned at epoch 10, current 100, ttl 0 ⇒ stale.
        let addr = fixture::addr("payload");
        let complete = fixture::complete_verified(&addr, CveDbEpoch(10));
        let ctx = VerifyContext::strict(CveDbEpoch(100));
        let err = verify(&complete, &ctx).expect_err("a stale scan must be fail-closed");
        assert!(
            err.iter()
                .any(|v| matches!(v, VerdictViolation::CveStale { .. }))
        );
    }

    #[test]
    fn vex_subtraction_admits_a_fully_suppressed_scan() {
        // B1: raw = 3 critical, but VEX suppresses all 3 ⇒ exploitable 0 ⇒
        // within a zero budget ⇒ admits. The world-class CVE⊖VEX gate.
        let addr = fixture::addr("payload");
        let mut complete = fixture::complete_verified(&addr, CveDbEpoch(100));
        complete.cve.raw = Severities::new(3, 5);
        complete.cve.vex_suppressed = Severities::new(3, 5);
        let ctx = VerifyContext::strict(CveDbEpoch(100));
        assert!(
            verify(&complete, &ctx).is_ok(),
            "fully VEX-suppressed scan must admit"
        );
    }

    #[test]
    fn an_exploitable_critical_over_budget_denies_the_run_token() {
        // raw 2 crit, VEX suppresses 1 ⇒ exploitable 1 crit > budget 0 ⇒ deny.
        let addr = fixture::addr("payload");
        let mut complete = fixture::complete_verified(&addr, CveDbEpoch(100));
        complete.cve.raw = Severities::new(2, 0);
        complete.cve.vex_suppressed = Severities::new(1, 0);
        let ctx = VerifyContext::strict(CveDbEpoch(100));
        let err = verify(&complete, &ctx).expect_err("an exploitable critical must be fail-closed");
        assert!(
            err.iter()
                .any(|v| matches!(v, VerdictViolation::CveExploitableOverBudget { .. }))
        );
    }

    #[test]
    fn violations_aggregate_not_short_circuit() {
        // Break three clauses at once; one call reports all three.
        let addr = fixture::addr("payload");
        let mut complete = fixture::complete_verified(&addr, CveDbEpoch(10)); // stale
        complete.signature.state = SlotState::Failed("bad sig".into()); // unverified
        complete.sbom.component_count = 0; // empty sbom
        let ctx = VerifyContext::strict(CveDbEpoch(100));
        let err = verify(&complete, &ctx).expect_err("multiple broken clauses");
        assert!(
            err.len() >= 3,
            "expected aggregated violations, got {err:?}"
        );
    }

    #[test]
    fn dedup_witness_observes_a_shared_hash() {
        // Two consumers of the SAME address ⇒ served(2) > computed(1).
        let shared = fixture::addr("shared-closure");
        let other = fixture::addr("other");
        let out = dedup_witness([&shared, &shared, &other]);
        assert_eq!(out.computed, 2);
        assert_eq!(out.served, 3);
        assert!(
            out.observed_dedup(),
            "a shared hash must be reused, not recomputed"
        );
    }
}
