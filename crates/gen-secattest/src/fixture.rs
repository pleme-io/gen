//! Synthetic security-verdict fixtures the checker tests + the verification
//! matrix exercise.
//!
//! gen-secattest depends only on `gen-pdc` (for the reused `ContentAddr`), so
//! it cannot call a live producer (cosign/trivy/sbomnix). These fixtures
//! mirror the SHAPE of a real per-hash verdict faithfully enough to exercise
//! every clause — a clean complete-verified-fresh verdict (admits), and the
//! per-clause failure fixtures (each denied).

use crate::verdict::{
    AttestationAddr, CompleteVerdict, CveDbEpoch, CveSlot, PartialVerdict, ProvenanceSlot,
    SbomFormat, SbomSlot, Severities, SignatureSlot, SlotState, SlsaLevel,
};

/// A canonical attestation address for a seed — the reused PDC merkle hash,
/// so the fixture addresses ARE valid content addresses (S2 seal inherited).
#[must_use]
pub fn addr(seed: &str) -> AttestationAddr {
    AttestationAddr::merkle(seed.as_bytes(), vec![])
}

/// A COMPLETE, fully-VERIFIED, FRESH verdict for `subject`, scanned at
/// `epoch` with zero exploitable findings and SLSA L3 — the happy path that
/// admits under [`crate::verdict::VerifyContext::strict`].
#[must_use]
pub fn complete_verified(subject: &AttestationAddr, epoch: CveDbEpoch) -> CompleteVerdict {
    CompleteVerdict {
        subject: subject.clone(),
        signature: SignatureSlot {
            subject: subject.clone(),
            state: SlotState::Verified,
            identity: "keyless://fulcio/ci@pleme.io".to_string(),
        },
        sbom: SbomSlot {
            subject: subject.clone(),
            state: SlotState::Verified,
            format: SbomFormat::CycloneDx { minor: 6 },
            component_count: 42,
        },
        cve: CveSlot {
            subject: subject.clone(),
            state: SlotState::Verified,
            scanned_epoch: epoch,
            raw: Severities::new(0, 0),
            vex_suppressed: Severities::new(0, 0),
        },
        provenance: ProvenanceSlot {
            subject: subject.clone(),
            state: SlotState::Verified,
            slsa_level: SlsaLevel::L3,
        },
    }
}

/// The complete-verified verdict as its PARTIAL (wire/builder) form — every
/// slot present, so `into_complete` succeeds. The matrix's "conformant" row.
#[must_use]
pub fn complete_partial(subject: &AttestationAddr, epoch: CveDbEpoch) -> PartialVerdict {
    let c = complete_verified(subject, epoch);
    PartialVerdict {
        subject: c.subject,
        signature: Some(c.signature),
        sbom: Some(c.sbom),
        cve: Some(c.cve),
        provenance: Some(c.provenance),
    }
}

/// An INCOMPLETE partial verdict missing exactly `omit` — the S1 gap fixture.
/// Every required kind gets a variant so the matrix can prove each omission
/// is caught. `omit` is a required kind; omitting it drops that slot.
#[must_use]
pub fn incomplete_partial(
    subject: &AttestationAddr,
    epoch: CveDbEpoch,
    omit: crate::verdict::VerdictKind,
) -> PartialVerdict {
    use crate::verdict::VerdictKind;
    let mut p = complete_partial(subject, epoch);
    match omit {
        VerdictKind::Signature => p.signature = None,
        VerdictKind::Sbom => p.sbom = None,
        VerdictKind::Cve => p.cve = None,
        VerdictKind::Provenance => p.provenance = None,
        // non-required kinds are not border fields; omitting them is a no-op
        // (they never contribute to completeness).
        VerdictKind::Vex | VerdictKind::Scorecard | VerdictKind::Vsa => {}
    }
    p
}

/// An `Attestable` test double — an artifact carrying a fixed address + a
/// partial verdict, so the trait's fail-closed `admit` can be exercised
/// end-to-end (S1→S5 in one call).
pub struct FixtureArtifact {
    pub addr: AttestationAddr,
    pub verdict: PartialVerdict,
}

impl crate::invariant::Attestable for FixtureArtifact {
    fn attestation_addr(&self) -> &AttestationAddr {
        &self.addr
    }
    fn verdict(&self) -> PartialVerdict {
        self.verdict.clone()
    }
}
