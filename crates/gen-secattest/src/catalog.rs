//! The verdict-KIND catalog (CATALOG REFLECTION) — the §4 verdict-store
//! schema, typed and self-describing.
//!
//! Every verdict kind the per-hash security verdict is composed of is a typed
//! entry here, carrying its completeness role, its time-boundedness, the
//! standardized in-toto v1 `predicateType`, the world-class best-in-class OSS
//! producer, its maturity gate, and its honest achievable tier.
//!
//! **The forcing rule (CATALOG REFLECTION + CLOSED-LOOP MASS-SYNTHESIS rule
//! 1):** adding a kind to [`crate::verdict::VerdictKind`] REQUIRES landing its
//! catalog entry here AND its matrix row in `tests/secattest_matrix.rs` in the
//! same commit. `every_kind_has_a_catalog_entry` fails the build when they
//! drift. The catalog IS the doc; the matrix IS the proof.
//!
//! **Tier-honest maturity (stack doc §5):** the CONTRACT (these types) ships
//! now, but every PRODUCER is DESIGN/unwired today — zero live callers on the
//! image pipeline, Kyverno suspended, the VEX Helm field inert. So every
//! catalog entry's maturity is `Design`; the histogram assertion IS the
//! ledger — promote a kind's maturity DELIBERATELY when its producer wires
//! live. No FedRAMP-certified claim; FIPS-CMVP validated-build + live 3PAO
//! are named external gates, never a maturity here.

use gen_pdc::Maturity;

use crate::clause::AttestTier;
use crate::verdict::VerdictKind;

/// A typed point in the verdict-store schema — one verdict kind.
#[derive(Clone, Debug)]
pub struct VerdictKindEntry {
    /// The verdict kind (the catalog's unique key).
    pub kind: VerdictKind,
    /// Required for S1 completeness? {signature, sbom, cve, provenance} = yes.
    pub required_for_completeness: bool,
    /// Timeless (dedup-eligible, no TTL) vs time-bound (cve/vex).
    pub timeless: bool,
    /// The standardized in-toto attestation-spec v1 `predicateType` URI.
    pub predicate_type: &'static str,
    /// The world-class best-in-class OSS producer for this kind (stack §3).
    pub best_in_class_tool: &'static str,
    /// PRODUCER maturity — the wiring, not the contract. DESIGN today.
    pub maturity: Maturity,
    /// The honest UNREPRESENTABILITY tier this kind's clause reaches at its
    /// destination (never rounded up past the clause's achievable ceiling).
    pub achievable_tier: AttestTier,
    /// One-line purpose.
    pub purpose: &'static str,
}

const fn entry(
    kind: VerdictKind,
    required_for_completeness: bool,
    timeless: bool,
    predicate_type: &'static str,
    best_in_class_tool: &'static str,
    achievable_tier: AttestTier,
    purpose: &'static str,
) -> VerdictKindEntry {
    VerdictKindEntry {
        kind,
        required_for_completeness,
        timeless,
        predicate_type,
        best_in_class_tool,
        // Tier-honest: every producer is DESIGN/unwired today (stack §5).
        maturity: Maturity::Design,
        achievable_tier,
        purpose,
    }
}

/// A1 — signature. Required, timeless. Nix-native ed25519 / cosign keyless.
const SIGNATURE: VerdictKindEntry = entry(
    VerdictKind::Signature,
    true,
    true,
    "https://in-toto.io/attestation/link/v0.3",
    "Nix ed25519 (sui CacheSigner) / cosign keyless (Fulcio, >=2.6.0)",
    AttestTier::TrulyUnrepLibraryParseWire,
    "store-path / OCI signature — poisoned-write rejected on consume",
);

/// A4 — SBOM. Required, timeless. sbomnix (closure) + Syft (OCI fallback).
const SBOM: VerdictKindEntry = entry(
    VerdictKind::Sbom,
    true,
    true,
    "https://cyclonedx.org/bom/v1.6",
    "sbomnix (closure, primary) + Syft (OCI boundary fallback)",
    AttestTier::TrulyUnrepLibraryParseWire,
    "component inventory (CM-8) — exact per-closure, dedup-composable",
);

/// A5 — CVE. Required, TIME-BOUND. Trivy (primary) + osv-scanner (2nd source).
const CVE: VerdictKindEntry = entry(
    VerdictKind::Cve,
    true,
    false,
    "https://in-toto.io/attestation/vulns/v0.2",
    "Trivy (primary) union osv-scanner (offline, lockfile axis)",
    AttestTier::CeilingC2,
    "flaw detection (RA-5) keyed (hash, cve_db_epoch) + TTL — the C2 ceiling",
);

/// B1 — VEX. Refines CVE (not a completeness slot). OpenVEX; gate = cve ⊖ vex.
const VEX: VerdictKindEntry = entry(
    VerdictKind::Vex,
    false,
    false,
    "https://openvex.dev/ns/v0.2.0",
    "OpenVEX (vexctl) + reachability",
    AttestTier::CeilingC2,
    "exploitability triage — the CVE gate reads cve ⊖ vex, not raw noise",
);

/// A6 — provenance. Required, timeless. SLSA Provenance v1 + in-toto/DSSE.
const PROVENANCE: VerdictKindEntry = entry(
    VerdictKind::Provenance,
    true,
    true,
    "https://slsa.dev/provenance/v1",
    "SLSA Provenance v1 + in-toto v1 / DSSE v1.0",
    AttestTier::TrulyUnrepLibraryParseWire,
    "build provenance (SR-3/4) — the SLSA-L3 build-integrity claim",
);

/// B4 — Scorecard. Build axis (not per-hash). OpenSSF Scorecard.
const SCORECARD: VerdictKindEntry = entry(
    VerdictKind::Scorecard,
    false,
    true,
    "https://slsa.dev/scorecard/v0.1",
    "OpenSSF Scorecard (v5.5.0+)",
    AttestTier::OnlyMitigated,
    "repo/build posture (SA-15) — orthogonal build-axis evidence beside provenance",
);

/// B3 — VSA. The capstone verdict-of-verdicts. slsa-verifier verify-vsa.
const VSA: VerdictKindEntry = entry(
    VerdictKind::Vsa,
    false,
    true,
    "https://slsa.dev/verification_summary/v1",
    "SLSA VSA (slsa-verifier verify-vsa)",
    AttestTier::CeilingC1,
    "the signed 'met SLSA-L3' summary — one object a consumer/3PAO checks once",
);

/// The full verdict-kind catalog. Order: required slots first (S1), then the
/// refinements (VEX, Scorecard, VSA) — stable for diffs. Adding a kind = a
/// `const` + one entry here + one matrix row (same commit; the matrix enforces).
pub const CATALOG: &[&VerdictKindEntry] =
    &[&SIGNATURE, &SBOM, &CVE, &PROVENANCE, &VEX, &SCORECARD, &VSA];

/// Look up a kind's catalog entry.
#[must_use]
pub fn entry_for(kind: VerdictKind) -> Option<&'static VerdictKindEntry> {
    CATALOG.iter().copied().find(|e| e.kind == kind)
}

/// The producer-maturity histogram — `(design, m1, shipped)`. Sums to
/// `CATALOG.len()` (partition-complete; the test asserts it).
#[must_use]
pub fn maturity_histogram() -> (usize, usize, usize) {
    let mut h = (0, 0, 0);
    for e in CATALOG {
        match e.maturity {
            Maturity::Design => h.0 += 1,
            Maturity::M1 => h.1 += 1,
            Maturity::Shipped => h.2 += 1,
        }
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_has_exactly_one_catalog_entry() {
        // CATALOG REFLECTION: the catalog ⇄ the VerdictKind partition. A new
        // kind cannot ship without an entry, and no entry names a phantom kind.
        for kind in VerdictKind::ALL {
            let n = CATALOG.iter().filter(|e| e.kind == kind).count();
            assert_eq!(
                n, 1,
                "kind {kind:?} must have exactly one catalog entry (found {n})"
            );
        }
        assert_eq!(
            CATALOG.len(),
            VerdictKind::ALL.len(),
            "catalog has extra/duplicate entries"
        );
    }

    #[test]
    fn required_flags_match_the_completeness_partition() {
        // The catalog's `required_for_completeness` must agree with the
        // typed `VerdictKind::REQUIRED` set — one source of truth, two witnesses.
        for e in CATALOG {
            assert_eq!(
                e.required_for_completeness,
                e.kind.required_for_completeness(),
                "catalog required flag drifted for {:?}",
                e.kind
            );
        }
        let required: Vec<_> = CATALOG
            .iter()
            .filter(|e| e.required_for_completeness)
            .map(|e| e.kind)
            .collect();
        assert_eq!(
            required.len(),
            4,
            "exactly four required completeness slots"
        );
    }

    #[test]
    fn timeless_flags_match_the_dedup_partition() {
        // Timeless ⟺ dedup-eligible. Only CVE + VEX are time-bound.
        for e in CATALOG {
            assert_eq!(
                e.timeless,
                e.kind.is_timeless(),
                "timeless drift for {:?}",
                e.kind
            );
        }
    }

    #[test]
    fn predicate_types_match_the_typed_kind() {
        // The catalog's in-toto predicateType must equal the typed one — the
        // envelope standardization stays single-sourced.
        for e in CATALOG {
            assert_eq!(
                e.predicate_type,
                e.kind.predicate_type(),
                "predicate drift for {:?}",
                e.kind
            );
        }
    }

    #[test]
    fn no_entry_rounds_its_tier_up_past_its_clause_ceiling() {
        // Honesty gate: a required slot's achievable tier must not exceed its
        // clause's achievable ceiling. Signature/SBOM/Provenance discharge S1
        // (Complete) → TrulyUnrepLibraryParseWire; CVE is time-bound → C2.
        use crate::clause::SecClause;
        for e in CATALOG {
            if !e.required_for_completeness {
                continue;
            }
            let ceiling = if e.timeless {
                SecClause::Complete.achievable_tier()
            } else {
                SecClause::FreshnessGated.achievable_tier()
            };
            assert!(
                e.achievable_tier.rank() <= ceiling.rank(),
                "kind {:?} rounds its tier UP: claims {:?}, ceiling {:?}",
                e.kind,
                e.achievable_tier,
                ceiling
            );
        }
    }

    #[test]
    fn maturity_histogram_is_the_honest_ledger() {
        // Tier-honest snapshot (2026-07): the CONTRACT ships; every PRODUCER
        // is DESIGN/unwired. This line IS the ledger — when a producer wires
        // live (e.g. sui cache signing, then B1 VEX + B2 OSV), promote its
        // maturity DELIBERATELY and update this assertion.
        let (design, m1, shipped) = maturity_histogram();
        assert_eq!(
            design + m1 + shipped,
            CATALOG.len(),
            "histogram must partition the catalog"
        );
        assert_eq!(
            (design, m1, shipped),
            (7, 0, 0),
            "SecAttest producer-maturity ledger drifted — update deliberately, never round up"
        );
    }
}
