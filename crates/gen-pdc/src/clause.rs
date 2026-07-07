//! The six PDC clauses + their honest UNREPRESENTABILITY tiers.
//!
//! The Per-Dependency Content-Addressed Cacheability invariant is ONE law
//! with six clauses. Each ecosystem variant discharges the same clauses;
//! today each does so with its own `Violation` enum (six independent
//! `check()` fns). This module names the law once so the duplication is
//! recognized as one thing with variant-local extensions.
//!
//! Canonical statement: `theory/PDC-CONTRACT.md` §2.

use serde::{Deserialize, Serialize};

/// The six clauses of the PDC invariant. A variant is a legal point on the
/// PDC lattice iff its emitted specs satisfy every clause.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PdcClause {
    /// **C1 Content-addressed.** Every buildable node carries a non-empty
    /// content address `addr(n) = H(source(n) ∪ {addr(m) | n→m})`, a pure
    /// deterministic function of its own source and its direct deps'
    /// addresses. (cargo `sha256`; gomod `source_hash`.)
    ContentAddressed,
    /// **C2 Edge-resolvable.** Every dependency named by an edge exists as
    /// a node in the same spec — no dangling edge. (cargo `UnresolvedDep`;
    /// gomod Go-I1.)
    EdgeResolvable,
    /// **C3 Dedup-by-address.** The interpreter computes each node exactly
    /// once and reuses it for every dependent (the `lib.fix`/`self`
    /// fixpoint). Two builds/consumers/runners sharing `addr(n)` share the
    /// build. This is a graph-reachability property — a CI ceiling (C1), not
    /// a compile error.
    DedupByAddress,
    /// **C4 Boundary-precise.** Editing a node re-derives only it and its
    /// transitive dependents; every unshared node is a cache hit. `addr(n)`
    /// is simultaneously the cache key, the incremental boundary, and the
    /// attestation address (one cell, not three).
    BoundaryPrecise,
    /// **C5 Kind ⟺ source.** A node's kind partitions its source class
    /// disjointly; a class a variant cannot build is STRUCTURALLY ABSENT
    /// from the kind enum, not runtime-guarded. (gomod Go-I10 + Go-I12;
    /// cargo Target/Host + proc_macro.)
    KindSourceDisjoint,
    /// **C6 Freshness is a gate.** A spec below the current schema version
    /// is a violation, and CI fails on any diff between the committed spec
    /// and a fresh `gen build .`. Stale specs are impossible by
    /// construction. (cargo/gomod `StaleSchemaVersion`.)
    FreshnessGated,
}

impl PdcClause {
    /// Every clause, in canonical order. The partition the matrix asserts.
    pub const ALL: [PdcClause; 6] = [
        PdcClause::ContentAddressed,
        PdcClause::EdgeResolvable,
        PdcClause::DedupByAddress,
        PdcClause::BoundaryPrecise,
        PdcClause::KindSourceDisjoint,
        PdcClause::FreshnessGated,
    ];

    /// The honest UNREPRESENTABILITY tier the clause CAN reach when a
    /// variant seals it at the destination (`theory/PDC-CONTRACT.md` §5).
    /// This is the *achievable* tier, not a claim any given variant is
    /// already there — a variant's per-clause status lives in the catalog.
    #[must_use]
    pub fn achievable_tier(self) -> UnrepTier {
        match self {
            // ContentAddr newtype: no empty-address code path (library),
            // Err at deserialize (wire).
            PdcClause::ContentAddressed => UnrepTier::TrulyUnrepLibraryParseWire,
            // kind enum with unbuildable arms absent (gomod already).
            PdcClause::KindSourceDisjoint => UnrepTier::TrulyUnrep,
            // dangling edge: still a value-level walk (a key COULD be absent
            // and detected). Runtime-checked; the honest ceiling for a
            // dynamic resolver graph.
            PdcClause::EdgeResolvable => UnrepTier::OnlyMitigated,
            // graph-reachability quantifier — no dependent-type proof in
            // Rust; a CI forcing-function. UNREP ceiling C1.
            PdcClause::DedupByAddress => UnrepTier::CeilingC1,
            PdcClause::BoundaryPrecise => UnrepTier::CeilingC1,
            // stale schema is a runtime comparison + a CI diff gate.
            PdcClause::FreshnessGated => UnrepTier::OnlyMitigated,
        }
    }
}

/// The tier ladder from `theory/UNREPRESENTABILITY.md` §II, plus the C1
/// ceiling. Never round up: a `Result::Err` is mitigation, a compile error
/// / absent path is unrepresentability.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UnrepTier {
    /// Compile error / no expressible path — the strongest tier.
    TrulyUnrep,
    /// truly-unrep in the Rust library + parse-time-rejected at the wire
    /// boundary (the `ContentAddr` shape).
    TrulyUnrepLibraryParseWire,
    /// `Err` at the deserialize boundary + sealed in-Rust construction.
    ParseTimeRejected,
    /// runtime lock/clamp/`if is_valid()` — a `Vec<Violation>` row.
    OnlyMitigated,
    /// Best-possible: no dependent-type reachability proof in Rust; a
    /// mechanical CI forcing-function is the correct terminal answer.
    CeilingC1,
}
