//! The security-CONCERN catalog (CATALOG REFLECTION) — the layered enjulho
//! ship as typed data.
//!
//! Where [`crate::catalog`] is the §4 **verdict-store schema** (what verdict
//! kinds compose the per-hash verdict — the DATA axis), this module is the
//! **layered ship** (what security CONCERNS the pipeline addresses, which
//! best-in-class OSS engine handles each, at what maturity, mapped to which
//! NIST 800-53 Rev 5 control, and where each sits in the six-layer
//! per-derivation pipeline — the DELIVERY axis). The two axes cross-witness
//! each other: exactly the four concerns whose failure denies the typed
//! run path (S3) are the four [`crate::verdict::VerdictKind::REQUIRED`]
//! verdict kinds. `no_verdict_slot_rounded_up` (in `tests/concern_matrix.rs`)
//! enforces that tie.
//!
//! **OSS-FIRST, no-NIH.** Every engine is an industry standard (Nix-native
//! ed25519 · Sigstore cosign/rekor · sbomnix/Syft · Trivy/osv-scanner ·
//! OpenVEX · SLSA/in-toto · GUAC · OpenSSF Scorecard · Kyverno verifyImages).
//! pleme-io content is reserved for the four *tested voids* the OSS stack
//! leaves open (air-gap inclusion verify · re-derivable proof · BLAKE3
//! store-binding · eBPF kernel-time enforcement); those sit BESIDE the
//! catalog, not inside it.
//!
//! **The forcing rule (CATALOG REFLECTION + CLOSED-LOOP MASS-SYNTHESIS rule
//! 1):** adding a concern to [`Concern`] REQUIRES landing its
//! [`ConcernSlot`] here AND its matrix row in `tests/concern_matrix.rs` in
//! the same commit. `matrix_covers_every_concern` fails the build on drift.

use crate::verdict::VerdictKind;

/// A security concern the layered per-derivation ship addresses — one typed
/// slot per concern. The eight concerns partition the enjulho surface
/// (`security.perDerivation` in helmworks `_compliance_perderivation.tpl`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Concern {
    /// L-sign — store-path / OCI signature (integrity).
    Signing,
    /// L-sbom — component inventory (CM-8).
    Sbom,
    /// L-cve/vex — flaw detection + exploitability triage (RA-5, SI-2).
    CveVex,
    /// L-provenance/vsa — build provenance + verification summary (SR-3/4).
    ProvenanceVsa,
    /// L-transparency — tamper-evident inclusion log (AU-2/12).
    Transparency,
    /// Correlation — cross-artifact knowledge graph (beside the pipeline).
    Guac,
    /// Posture — repo/build security posture (beside the pipeline; SA-15).
    Scorecard,
    /// L-admission — runtime cluster gate on the verdict annotations.
    Admission,
}

impl Concern {
    /// Every concern, in canonical (pipeline-then-beside) order.
    pub const ALL: [Concern; 8] = [
        Concern::Signing,
        Concern::Sbom,
        Concern::CveVex,
        Concern::ProvenanceVsa,
        Concern::Transparency,
        Concern::Guac,
        Concern::Scorecard,
        Concern::Admission,
    ];

    /// The verdict kind this concern PRODUCES into the §4 store, if it maps
    /// to a required completeness slot. Only the four fail-closed concerns
    /// map to a required verdict kind — the tie the honesty gate enforces.
    #[must_use]
    pub fn required_verdict_kind(self) -> Option<VerdictKind> {
        match self {
            Concern::Signing => Some(VerdictKind::Signature),
            Concern::Sbom => Some(VerdictKind::Sbom),
            Concern::CveVex => Some(VerdictKind::Cve),
            Concern::ProvenanceVsa => Some(VerdictKind::Provenance),
            Concern::Transparency | Concern::Guac | Concern::Scorecard | Concern::Admission => None,
        }
    }
}

/// Where a concern sits in the layered ship. The first six are the ORDERED
/// per-derivation pipeline (L-sign → … → L-admission); `Posture` and
/// `Correlation` sit BESIDE it (build-axis / cross-artifact, not per-hash).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Layer {
    Sign,
    Sbom,
    CveVex,
    ProvenanceVsa,
    Transparency,
    Admission,
    /// Beside — build-axis posture (Scorecard).
    Posture,
    /// Beside — cross-artifact correlation (GUAC).
    Correlation,
}

impl Layer {
    /// The ordinal in the six-layer pipeline (`Some(0..=5)`), or `None` for a
    /// beside-the-pipeline layer (Posture / Correlation).
    #[must_use]
    pub fn pipeline_order(self) -> Option<u8> {
        match self {
            Layer::Sign => Some(0),
            Layer::Sbom => Some(1),
            Layer::CveVex => Some(2),
            Layer::ProvenanceVsa => Some(3),
            Layer::Transparency => Some(4),
            Layer::Admission => Some(5),
            Layer::Posture | Layer::Correlation => None,
        }
    }
}

/// Shipping maturity of a concern's live ENFORCEMENT (the wiring, not the
/// contract). Distinct from `gen_pdc::Maturity` (which grades a per-package
/// encoder): here **Wired** = the concern's OSS engine is threaded into a
/// live surface (the super-cache-ci pipeline step or the helmworks overlay)
/// but its end-to-end run is a named LiveTODO; **Shipped** = enforced live +
/// proven end-to-end (nothing reaches this today — the honest ceiling).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ShipMaturity {
    /// Contract typed, but no live caller — the engine is not yet threaded.
    Design,
    /// The engine is threaded into a live surface (pipeline step / overlay);
    /// the end-to-end enforcing run is a named LiveTODO.
    Wired,
    /// Enforced live + proven end-to-end. Nothing is here today (the ceiling).
    Shipped,
}

impl ShipMaturity {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            ShipMaturity::Design => "design",
            ShipMaturity::Wired => "wired",
            ShipMaturity::Shipped => "shipped",
        }
    }
}

/// One typed point in the layered security ship — a concern's slot.
#[derive(Clone, Debug)]
pub struct ConcernSlot {
    /// The concern (the catalog's unique key).
    pub concern: Concern,
    /// The best-in-class OSS engine(s) that realize this concern (no-NIH).
    pub engine: &'static str,
    /// Live-enforcement maturity — tier-honest, never rounded up.
    pub maturity: ShipMaturity,
    /// The NIST 800-53 Rev 5 controls this concern discharges. NONEMPTY for
    /// every concern (the `control-mapped` matrix assertion).
    pub controls: &'static [&'static str],
    /// Where the concern sits in the layered ship.
    pub layer: Layer,
    /// Does a FAILURE of this concern deny the typed run path (S3)? True IFF
    /// the concern maps to a required verdict kind the `verify()` law gates
    /// on. Advisory/evidence/beside concerns are `false` — and MUST NOT claim
    /// `true` (the `no_verdict_slot_rounded_up` gate).
    pub fail_closed: bool,
    /// One-line purpose.
    pub purpose: &'static str,
}

const fn slot(
    concern: Concern,
    engine: &'static str,
    maturity: ShipMaturity,
    controls: &'static [&'static str],
    layer: Layer,
    fail_closed: bool,
    purpose: &'static str,
) -> ConcernSlot {
    ConcernSlot {
        concern,
        engine,
        maturity,
        controls,
        layer,
        fail_closed,
        purpose,
    }
}

// ── The eight concern slots ──────────────────────────────────────────────
// Maturity is tier-honest (stack doc §5 + this workflow's phase 1): the
// contract ships; Signing/Sbom/CveVex/ProvenanceVsa/Transparency engines are
// WIRED into the super-cache-ci pipeline (their actions were zero-caller
// before; now threaded) — end-to-end enforcing run is a named LiveTODO;
// Guac/Scorecard/Admission are DESIGN (M2/M1 beside-the-pipeline, Kyverno
// suspended/Audit on rio). NOTHING is Shipped (the honest ceiling).

const SIGNING: ConcernSlot = slot(
    Concern::Signing,
    "Nix ed25519 (sui CacheSigner, live-reject proven) + Sigstore cosign keyless (Fulcio, >=2.6.0)",
    ShipMaturity::Wired,
    &["SI-7", "CM-14", "SR-11"],
    Layer::Sign,
    true,
    "store-path / OCI signature — a poisoned write is rejected on consume",
);

const SBOM: ConcernSlot = slot(
    Concern::Sbom,
    "sbomnix (closure-driven, primary) + Syft (OCI fallback) — CycloneDX 1.6",
    ShipMaturity::Wired,
    &["CM-8", "SR-3", "SR-4"],
    Layer::Sbom,
    true,
    "component inventory (CM-8) — exact per-closure, dedup-composable",
);

const CVE_VEX: ConcernSlot = slot(
    Concern::CveVex,
    "Trivy (primary) + osv-scanner (2nd source, offline) + OpenVEX (vexctl triage)",
    ShipMaturity::Wired,
    &["RA-5", "SI-2"],
    Layer::CveVex,
    true,
    "flaw detection — the gate reads cve ⊖ vex (exploitable), not raw noise",
);

const PROVENANCE_VSA: ConcernSlot = slot(
    Concern::ProvenanceVsa,
    "SLSA Provenance v1 + in-toto v1/DSSE + SLSA VSA (slsa-verifier verify-vsa)",
    ShipMaturity::Wired,
    &["SR-3", "SR-4", "SA-10", "SR-11"],
    Layer::ProvenanceVsa,
    true,
    "build provenance (SR-3/4) + the signed 'met SLSA-L3' verdict-of-verdicts",
);

const TRANSPARENCY: ConcernSlot = slot(
    Concern::Transparency,
    "Rekor v2 (tile-backed, via cosign keyless) + cartorio offline proof (air-gap void)",
    ShipMaturity::Wired,
    &["AU-2", "AU-12"],
    Layer::Transparency,
    false,
    "tamper-evident inclusion log — online rekor; offline-tameshi is the IL6 void",
);

const GUAC: ConcernSlot = slot(
    Concern::Guac,
    "GUAC (OpenSSF incubating) — cross-artifact knowledge graph",
    ShipMaturity::Design,
    &["RA-3", "RA-5"],
    Layer::Correlation,
    false,
    "fleet-scale blast-radius correlation — M2, beside the per-hash store",
);

const SCORECARD: ConcernSlot = slot(
    Concern::Scorecard,
    "OpenSSF Scorecard (v5.5.0+) — repo/build posture score",
    ShipMaturity::Design,
    &["SA-15", "SA-11"],
    Layer::Posture,
    false,
    "repo/build posture (SA-15) — build-axis evidence, orthogonal to the hash",
);

const ADMISSION: ConcernSlot = slot(
    Concern::Admission,
    "Kyverno verifyImages (Enforce) / Sigstore policy-controller",
    ShipMaturity::Design,
    &["CM-14", "AC-3", "SR-11"],
    Layer::Admission,
    false,
    "runtime cluster gate on the verdict annotations — Enforce (Kyverno suspended on rio = Design)",
);

/// The full concern catalog — the layered ship as typed data. Pipeline
/// layers first (in order), then the beside-the-pipeline concerns. Adding a
/// concern = a `const` + one entry here + one matrix row (same commit).
pub const CONCERN_CATALOG: &[&ConcernSlot] = &[
    &SIGNING,
    &SBOM,
    &CVE_VEX,
    &PROVENANCE_VSA,
    &TRANSPARENCY,
    &ADMISSION,
    &SCORECARD,
    &GUAC,
];

/// Look up a concern's slot.
#[must_use]
pub fn concern_slot(concern: Concern) -> Option<&'static ConcernSlot> {
    CONCERN_CATALOG.iter().copied().find(|s| s.concern == concern)
}

/// The live-enforcement maturity histogram — `(design, wired, shipped)`.
/// Sums to `CONCERN_CATALOG.len()`. This line IS the honest ledger; promote a
/// concern's maturity DELIBERATELY when its enforcement ships end-to-end.
#[must_use]
pub fn maturity_histogram() -> (usize, usize, usize) {
    let mut h = (0, 0, 0);
    for s in CONCERN_CATALOG {
        match s.maturity {
            ShipMaturity::Design => h.0 += 1,
            ShipMaturity::Wired => h.1 += 1,
            ShipMaturity::Shipped => h.2 += 1,
        }
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_concern_has_exactly_one_slot() {
        for c in Concern::ALL {
            let n = CONCERN_CATALOG.iter().filter(|s| s.concern == c).count();
            assert_eq!(n, 1, "concern {c:?} must have exactly one slot (found {n})");
        }
        assert_eq!(
            CONCERN_CATALOG.len(),
            Concern::ALL.len(),
            "catalog has extra/duplicate concern slots"
        );
    }

    #[test]
    fn the_six_pipeline_layers_are_present_and_ordered() {
        // The wired ship is L-sign(0) → L-sbom(1) → L-cve/vex(2) →
        // L-provenance/vsa(3) → L-transparency(4) → L-admission(5).
        let mut orders: Vec<u8> = CONCERN_CATALOG
            .iter()
            .filter_map(|s| s.layer.pipeline_order())
            .collect();
        orders.sort_unstable();
        assert_eq!(orders, vec![0, 1, 2, 3, 4, 5], "the six ordered pipeline layers must all be present exactly once");
    }

    #[test]
    fn fail_closed_concerns_are_exactly_the_required_verdict_kinds() {
        // The cross-witness: a concern is fail-closed IFF it produces a
        // required verdict kind the typed verify() law gates on. This is the
        // no-round-up tie — an advisory concern cannot claim fail-closed.
        for s in CONCERN_CATALOG {
            assert_eq!(
                s.fail_closed,
                s.concern.required_verdict_kind().is_some(),
                "concern {:?} fail_closed={} but required_verdict_kind={:?} — a concern is fail-closed IFF it maps to a required verdict kind",
                s.concern,
                s.fail_closed,
                s.concern.required_verdict_kind()
            );
        }
        let fc = CONCERN_CATALOG.iter().filter(|s| s.fail_closed).count();
        assert_eq!(fc, 4, "exactly four fail-closed concerns (the four required verdict kinds)");
    }

    #[test]
    fn every_concern_is_control_mapped() {
        // control-mapped: no concern ships without a NIST 800-53 crosswalk.
        for s in CONCERN_CATALOG {
            assert!(!s.controls.is_empty(), "concern {:?} has no 800-53 control mapping", s.concern);
            assert!(!s.engine.is_empty(), "concern {:?} has no named OSS engine", s.concern);
        }
    }

    #[test]
    fn maturity_ledger_is_the_honest_snapshot() {
        // Tier-honest (2026-07): the CONTRACT ships; the five pipeline engines
        // are WIRED (threaded into super-cache-ci-build.yml — zero callers
        // before); GUAC/Scorecard/Admission are DESIGN. NOTHING is Shipped
        // (the honest ceiling — no live-enforced + e2e-proven concern yet).
        let (design, wired, shipped) = maturity_histogram();
        assert_eq!(design + wired + shipped, CONCERN_CATALOG.len(), "histogram must partition the catalog");
        assert_eq!(
            (design, wired, shipped),
            (3, 5, 0),
            "SecAttest concern-maturity ledger drifted — update deliberately, never round up"
        );
    }
}
