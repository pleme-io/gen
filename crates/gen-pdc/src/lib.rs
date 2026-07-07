//! gen-pdc — the Per-Dependency Content-Addressed Cacheability contract.
//!
//! ## What this crate is
//!
//! The per-dependency-caching law already exists **implicitly** across two
//! shipped/near-shipped gen ecosystems — gen-cargo (shipped reference,
//! `RegistryWithoutSha256`) and gen-gomod (M1, Go-I8 `NodeMissingSourceHash`
//! — the literal per-dependency-caching invariant, named) — and their two
//! substrate interpreters (`rust/lockfile-builder.nix` and
//! `go/package-graph.nix`) run the **identical** `lib.fix (self: mapAttrs …)`
//! per-node dedup fixpoint. gen-pdc CODIFIES that law:
//!
//! 1. **The universal PDC invariant** ([`clause`]) — six clauses, named
//!    once, with their honest UNREPRESENTABILITY tiers. The six per-ecosystem
//!    `check()` fns are then recognized as ONE law with variant-local
//!    extensions.
//! 2. **The typed variant CATALOG** ([`catalog`], CATALOG REFLECTION) —
//!    every ecosystem is a typed point on the PDC lattice with a maturity
//!    gate (Design/M1/Shipped), a triplet-leg reference, an admitted
//!    source-class set, and a tier-honest per-clause status. Adding an
//!    ecosystem REQUIRES its catalog entry.
//! 3. **The universal PDC checker** ([`check`]) — one `check(spec,
//!    admitted)` over the ecosystem-agnostic [`spec::PdcSpec`] projection,
//!    running the edge-resolvable walk, the lazy memoized dedup fixpoint
//!    (clause 1 Merkle-agreement + clause 3 "computed once" witness), the
//!    kind⟺source partition, and the freshness gate.
//! 4. **The content-address SEAL** ([`content_addr::ContentAddr`]) — the
//!    refined type that moves clause 1 from *only-mitigated* (a runtime
//!    `Violation` in every ecosystem) to *truly-unrepresentable (library) /
//!    parse-time-rejected (wire)*. gen-pdc ships the type; the per-adapter
//!    border migration is the burn-down.
//!
//! The verification MATRIX (one row per ecosystem, failing the build when a
//! new variant lands without a row) lives in `tests/pdc_matrix.rs`.
//!
//! ## What this crate is NOT
//!
//! gen-pdc owns **no new engine** — the encoders, the interpreters, and the
//! dedup algorithm already exist. It is a **naming + de-duplication +
//! tier-promotion** of an already-lived pattern. It depends only on
//! `gen-types` (the stable trait surface), never on a per-ecosystem adapter
//! crate, so it is decoupled from any one adapter's churn (notably
//! gen-gomod's M1 encoder, under active development).
//!
//! Canonical spec: `theory/PDC-CONTRACT.md`. Composes GEN-TYPED-SPEC-CONTRACT,
//! CATALOG REFLECTION, CLOSED-LOOP MASS-SYNTHESIS (rule 1), UNREPRESENTABILITY,
//! and SUPER-CACHE-CI.

#![allow(clippy::module_name_repetitions)]

pub mod catalog;
pub mod check;
pub mod clause;
pub mod content_addr;
pub mod fixture;
pub mod spec;

pub use catalog::{
    maturity_histogram, variant, ClauseStatus, EcosystemVariant, Maturity, CATALOG,
};
pub use check::{check, PdcCheckOutcome, PdcViolation};
pub use clause::{PdcClause, UnrepTier};
pub use content_addr::{ContentAddr, ContentAddrError};
pub use spec::{PdcNode, PdcSpec, SourceClass};
