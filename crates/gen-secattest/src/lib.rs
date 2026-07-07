//! gen-secattest — the Security-Attestation (SecAttest) INVARIANT contract.
//!
//! ## The law (ecosystem/concern-agnostic)
//!
//! > **Every derivation carries a COMPLETE, content-addressed, FAIL-CLOSED
//! > security verdict — {signature, SBOM, CVE⊖VEX, provenance} keyed by the
//! > SAME content hash PDC uses — verified before use; an unverified /
//! > incomplete / tampered artifact has NO run path.**
//!
//! This is the security peer of `gen-pdc`. Where PDC codifies
//! per-dependency *cacheability* over a content-addressed Merkle DAG,
//! gen-secattest codifies per-derivation *provable security* over the SAME
//! addresses — the "attestation address" is the THIRD use of PDC clause-4's
//! one cell (`addr(n)` = cache key = incremental boundary = attestation
//! address). The address type is REUSED, not forked: [`AttestationAddr`] IS
//! `gen_pdc::ContentAddr`, so an empty/malformed address is already
//! unrepresentable (S2 inherits PDC clause 1).
//!
//! ## What this crate is
//!
//! A typed CONTRACT — a trait + border types + the fail-closed checker —
//! that OSS supply-chain tools (cosign/rekor, sbomnix/Syft, Trivy/osv-scanner,
//! OpenVEX, SLSA/in-toto, OpenSSF Scorecard, SLSA VSA, Kyverno) plug into as
//! *producers of one typed verdict*, not six ad-hoc gates. OSS-first,
//! no-NIH: every LAYER is a standard tool; this crate is only the typed
//! LAW + the four tested pleme-io voids (air-gap verify / re-derivable proof
//! / BLAKE3-store binding / eBPF), which sit BESIDE the verdict store.
//!
//! 1. **The six-clause SecAttest invariant** ([`clause`]) — S1 Complete ·
//!    S2 Attestation-addressed · S3 Fail-closed · S4 Subject-bound · S5
//!    Freshness-gated · S6 Dedup-by-address — each with its honest
//!    UNREPRESENTABILITY tier.
//! 2. **The typed verdict border + sealed capability** ([`verdict`]) — the
//!    four required slots, the incomplete→complete parse boundary (S1), and
//!    [`RunToken`], the proof-carrying run capability whose only minter is a
//!    passing verify (S3, fail-closed, truly-unrep).
//! 3. **The fail-closed checker + contract trait** ([`invariant`]) — the
//!    universal `verify` (S3/S4/S5), the [`Attestable`] trait's provided
//!    `admit` (S1→S5 in one call), and the S6 dedup witness (`CeilingC1`).
//! 4. **The verdict-kind CATALOG** ([`catalog`], CATALOG REFLECTION) — the
//!    §4 verdict-store schema (the DATA axis), self-describing + matrix-enforced.
//! 5. **The security-CONCERN CATALOG** ([`concern`], CATALOG REFLECTION) — the
//!    layered enjulho ship (the DELIVERY axis): eight typed concern slots
//!    (signing/SBOM/CVE⊖VEX/provenance+VSA/transparency/GUAC/Scorecard/
//!    admission), each with its best-in-class OSS engine, `Shipped/Wired/
//!    Design` maturity, and NIST 800-53 crosswalk. The four fail-closed
//!    concerns cross-witness the four required verdict kinds.
//!
//! The verification MATRICES (one row per verdict kind / per concern, failing
//! the build when one lands without a row) live in `tests/secattest_matrix.rs`
//! + `tests/concern_matrix.rs` (`matrix_covers_every_concern` +
//! `no_verdict_slot_rounded_up`).
//!
//! ## Named EXTERNAL gates (never claimed done)
//!
//! - **Kernel-time exec enforcement** — the OS refusing to `execve` bytes
//!   without a [`RunToken`] — is the eBPF void; DESIGN. Within the typed
//!   pipeline, "no run path without a token" is truly-unrepresentable.
//! - **`SlotState::Verified` reflecting a REAL crypto pass** rests on the
//!   producer's honesty + (for the hardest tier) a hardware attestation root
//!   this contract does not yet carry; DESIGN.
//! - **FIPS-140 CMVP validated *running build*** and a **live 3PAO pass**
//!   are external gates — no FedRAMP-certified claim is made here.
//!
//! Composes GEN-TYPED-SPEC-CONTRACT, CATALOG REFLECTION, CLOSED-LOOP
//! MASS-SYNTHESIS (rule 1), UNREPRESENTABILITY, and the PDC one-cell.

#![allow(clippy::module_name_repetitions)]

pub mod catalog;
pub mod clause;
pub mod concern;
pub mod fixture;
pub mod invariant;
pub mod verdict;

pub use catalog::{entry_for, maturity_histogram, VerdictKindEntry, CATALOG};
pub use concern::{
    concern_slot, Concern, ConcernSlot, Layer, ShipMaturity, CONCERN_CATALOG,
};
pub use clause::{AttestTier, SecClause};
pub use invariant::{
    dedup_witness, verify, Attestable, DedupOutcome, VerdictViolation,
};
pub use verdict::{
    AttestationAddr, CompleteVerdict, CveDbEpoch, CveSlot, EpochTtl, IncompleteVerdict,
    PartialVerdict, ProvenanceSlot, RunToken, SbomFormat, SbomSlot, Severities, SignatureSlot,
    SlotState, SlsaLevel, VerdictKind, VerifyContext,
};

/// The six-clause SecAttest invariant, one line each — the human-readable
/// form of the doctrine header ([`clause::SecClause`] is the typed form).
/// The doctrine-parity test binds the prose statement to the [`SecClause`]
/// enum so they can never drift apart in count or order.
pub const SEC_CLAUSES: [&str; 6] = [
    "1. complete: every verdict carries all four required slots {signature, sbom, cve⊖vex, provenance}",
    "2. attestation-addressed: every verdict is keyed by the PDC content hash (the third use of the one cell)",
    "3. fail-closed: an unverified/incomplete artifact has NO run path — the run token has no constructor but a passing verify",
    "4. subject-bound: every slot attests the SAME content hash as the artifact; a foreign (replayed) subject is rejected",
    "5. freshness-gated: the cve slot is keyed (hash, cve_db_epoch)+TTL; a stale scan against a newer epoch is a violation",
    "6. dedup-by-address: a timeless verdict is computed once per content hash and reused by every consumer (PDC CeilingC1)",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prose_clauses_and_typed_clauses_agree_on_count_and_order() {
        // The doctrine-parity gate: the human-readable clause strings and the
        // typed `SecClause` partition stay in lock-step. Add a clause to one
        // and not the other → this fails; the prose can never round up/down
        // past the typed enum.
        assert_eq!(SEC_CLAUSES.len(), SecClause::ALL.len());
        assert_eq!(SEC_CLAUSES.len(), 6);
        let expected_prefixes = ["1. ", "2. ", "3. ", "4. ", "5. ", "6. "];
        for (i, clause) in SecClause::ALL.iter().enumerate() {
            // Every clause has an achievable tier declared (no gap).
            let _ = clause.achievable_tier();
            assert!(
                SEC_CLAUSES[i].starts_with(expected_prefixes[i]),
                "prose clause {} must be 1-indexed to its typed position",
                i + 1
            );
        }
    }

    #[test]
    fn the_attestation_address_is_literally_the_pdc_content_hash() {
        // S2 / the one-cell: AttestationAddr IS gen_pdc::ContentAddr — the
        // reuse (not fork) that makes the address the SAME cell PDC keys on.
        // A merkle hash round-trips through the strict PDC digest parser, and
        // an empty address is rejected by the inherited parse-boundary seal.
        let a: AttestationAddr = fixture::addr("payload");
        assert!(gen_pdc::ContentAddr::parse_digest(a.as_str()).is_ok());
        assert!(gen_pdc::ContentAddr::try_parse("").is_err());
    }
}
