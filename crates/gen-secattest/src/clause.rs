//! The six SecAttest clauses + their honest UNREPRESENTABILITY tiers.
//!
//! The Security-Attestation (SecAttest) invariant is ONE law with six
//! clauses. Every artifact-producing surface (a Nix derivation, an OCI
//! image, a crate) discharges the same clauses against the SAME content
//! hash PDC (`gen-pdc`) already keys on. This module names the law once so
//! the per-concern tools (cosign, sbomnix, Trivy+osv, SLSA/in-toto, Kyverno)
//! are recognized as producers of ONE typed verdict, not six ad-hoc gates.
//!
//! Canonical statement: `scratchpad/security-worldclass-stack.md` §3–§4.
//! Composes GEN-TYPED-SPEC-CONTRACT, CATALOG REFLECTION, CLOSED-LOOP
//! MASS-SYNTHESIS (rule 1), UNREPRESENTABILITY, and the PDC one-cell (the
//! attestation address IS PDC clause-4's content hash — its third use).

use serde::{Deserialize, Serialize};

/// The six clauses of the SecAttest invariant. An artifact is admissible
/// (has a run path) iff its per-hash security verdict satisfies every clause.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SecClause {
    /// **S1 Complete.** Every verdict carries all four REQUIRED attestation
    /// slots — {signature, SBOM, CVE⊖VEX, provenance}. A missing slot is
    /// unrepresentable in the `CompleteVerdict` product type (no `Option`);
    /// the wire/builder form is parse-time-rejected (`into_complete` /
    /// `CompleteVerdict::deserialize`).
    Complete,
    /// **S2 Attestation-addressed.** Every verdict is keyed by the PDC
    /// content hash — the "attestation address" is `gen_pdc::ContentAddr`
    /// (the THIRD use of PDC clause-4's one cell: cache key = incremental
    /// boundary = attestation address). An empty/malformed address inherits
    /// PDC clause-1's seal (truly-unrep library / parse-rejected wire).
    AttestationAddressed,
    /// **S3 Fail-closed.** An unverified/incomplete artifact has NO run
    /// path. The run capability (`RunToken`) has no public constructor —
    /// the ONLY minter is a passing `verify()`. Absence of a verification
    /// is treated as failure, never as success.
    FailClosed,
    /// **S4 Subject-bound.** Every slot's attestation subject equals the
    /// artifact's own attestation address; a slot attesting a DIFFERENT
    /// content hash (a lifted/replayed attestation) is rejected. The binding
    /// is a typed `ContentAddr` comparison — parse-time-rejectable.
    SubjectBound,
    /// **S5 Freshness-gated.** The time-bound CVE slot is keyed
    /// `(content_hash, cve_db_epoch)` + a TTL. A scan older than the TTL
    /// against the current epoch is stale — a violation — even for unchanged
    /// bytes (the honest external-world C2 ceiling: new CVEs are disclosed
    /// against bytes that did not change).
    FreshnessGated,
    /// **S6 Dedup-by-address.** A TIMELESS verdict (signature/SBOM/
    /// provenance) is a pure function of the bytes — computed once per
    /// content hash and reused by every consumer (build-once-reuse). A
    /// graph-reachability quantifier ⇒ a CI forcing-function (inherits PDC
    /// clause-3/4 `CeilingC1`), never a compile error.
    DedupByAddress,
}

impl SecClause {
    /// Every clause, in canonical order. The partition the matrix asserts.
    pub const ALL: [SecClause; 6] = [
        SecClause::Complete,
        SecClause::AttestationAddressed,
        SecClause::FailClosed,
        SecClause::SubjectBound,
        SecClause::FreshnessGated,
        SecClause::DedupByAddress,
    ];

    /// The honest UNREPRESENTABILITY tier the clause CAN reach at its
    /// destination. The *achievable* tier — not a claim the shipped wiring
    /// is already there (the producers are all DESIGN today; see
    /// [`crate::catalog`]).
    #[must_use]
    pub fn achievable_tier(self) -> AttestTier {
        match self {
            // Product-with-no-Option (library) + into_complete rejects (wire).
            SecClause::Complete => AttestTier::TrulyUnrepLibraryParseWire,
            // Inherits ContentAddr's parse-boundary seal (PDC clause 1).
            SecClause::AttestationAddressed => AttestTier::TrulyUnrepLibraryParseWire,
            // Proof-carrying capability: no RunToken path but a passing
            // verify. (Kernel-time execve enforcement is a named EXTERNAL
            // ceiling — the eBPF void the stack doc §Tier-C names — DESIGN.)
            SecClause::FailClosed => AttestTier::TrulyUnrep,
            // A typed ContentAddr equality, rejectable at the parse boundary.
            SecClause::SubjectBound => AttestTier::ParseTimeRejected,
            // Runtime epoch/TTL comparison; new CVEs vs unchanged bytes is an
            // external-world fact — best-possible-runtime (C2), never a type.
            SecClause::FreshnessGated => AttestTier::CeilingC2,
            // Graph-reachability "computed once" — a CI forcing-function
            // (C1), inherits PDC. No dependent-type proof in Rust.
            SecClause::DedupByAddress => AttestTier::CeilingC1,
        }
    }
}

/// The UNREPRESENTABILITY tier ladder (`theory/UNREPRESENTABILITY.md` §II),
/// a SUPERSET of `gen_pdc::UnrepTier` — it adds the external-world **C2**
/// ceiling PDC did not need (crypto validity + CVE freshness are runtime
/// facts of the world, distinct from C1's graph-reachability). Never round
/// up: a `Result::Err` is mitigation; a compile error / absent path is
/// unrepresentability; a ceiling is "as good as it gets" for its clause.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AttestTier {
    /// Compile error / no expressible path — the strongest tier.
    TrulyUnrep,
    /// truly-unrep in the Rust library + parse-time-rejected at the wire
    /// boundary (the `ContentAddr` / `CompleteVerdict` shape).
    TrulyUnrepLibraryParseWire,
    /// `Err` at the deserialize boundary + sealed in-Rust construction.
    ParseTimeRejected,
    /// runtime lock/clamp/`if is_valid()` — a `Vec<Violation>` row.
    OnlyMitigated,
    /// Best-possible: no dependent-type reachability proof in Rust; a
    /// mechanical CI forcing-function is the correct terminal answer.
    /// (dedup / boundary — the PDC C1 ceiling.)
    CeilingC1,
    /// Best-possible: an EXTERNAL-WORLD fact (a crypto verification result,
    /// a CVE disclosed after the scan) that no amount of typing derives from
    /// the bytes; a runtime check + continuous re-attestation is the correct
    /// terminal answer. Distinct from C1 (which is a reachability *proof*
    /// gap, not a world-observation gap).
    CeilingC2,
}

impl AttestTier {
    /// Strength rank — for the "no clause rounds its tier up past its
    /// achievable ceiling" gate. A ceiling is placed at its
    /// best-possible-for-its-clause position; a stronger tier ranks higher.
    #[must_use]
    pub fn rank(self) -> u8 {
        match self {
            AttestTier::OnlyMitigated => 0,
            AttestTier::CeilingC2 => 1,
            AttestTier::CeilingC1 => 2,
            AttestTier::ParseTimeRejected => 3,
            AttestTier::TrulyUnrepLibraryParseWire => 4,
            AttestTier::TrulyUnrep => 5,
        }
    }
}

impl From<gen_pdc::UnrepTier> for AttestTier {
    /// The bridge proving this ladder is a superset of PDC's — the two
    /// vocabularies compose, so an inherited PDC tier (e.g. the dedup
    /// `CeilingC1`) maps cleanly into a SecAttest clause status.
    fn from(t: gen_pdc::UnrepTier) -> Self {
        match t {
            gen_pdc::UnrepTier::TrulyUnrep => AttestTier::TrulyUnrep,
            gen_pdc::UnrepTier::TrulyUnrepLibraryParseWire => {
                AttestTier::TrulyUnrepLibraryParseWire
            }
            gen_pdc::UnrepTier::ParseTimeRejected => AttestTier::ParseTimeRejected,
            gen_pdc::UnrepTier::OnlyMitigated => AttestTier::OnlyMitigated,
            gen_pdc::UnrepTier::CeilingC1 => AttestTier::CeilingC1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_clause_has_an_achievable_tier() {
        for c in SecClause::ALL {
            // No gap — every clause declares its destination tier.
            let _ = c.achievable_tier();
        }
        assert_eq!(SecClause::ALL.len(), 6);
    }

    #[test]
    fn pdc_tier_ladder_embeds_into_the_attest_ladder() {
        // The superset property: every PDC tier maps to a SecAttest tier of
        // the SAME name (no re-grading), so the C1 dedup inheritance is exact.
        assert_eq!(
            AttestTier::from(gen_pdc::UnrepTier::CeilingC1),
            AttestTier::CeilingC1
        );
        assert_eq!(
            AttestTier::from(gen_pdc::UnrepTier::TrulyUnrepLibraryParseWire),
            AttestTier::TrulyUnrepLibraryParseWire
        );
    }

    #[test]
    fn fail_closed_is_the_strongest_clause() {
        // S3 is the load-bearing invariant — it must be truly-unrep (the
        // proof-carrying capability), the top of the ladder.
        assert_eq!(
            SecClause::FailClosed.achievable_tier(),
            AttestTier::TrulyUnrep
        );
        assert_eq!(AttestTier::TrulyUnrep.rank(), 5);
    }
}
