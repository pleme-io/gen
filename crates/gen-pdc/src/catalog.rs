//! The PDC variant CATALOG (CATALOG REFLECTION).
//!
//! Every ecosystem that participates in per-dependency caching is a typed
//! point on the PDC lattice, declared here as an [`EcosystemVariant`]. The
//! catalog is the self-describing object: tooling (`gen confirm
//! --all-ecosystems`, generated docs, the verification matrix) iterates it
//! mechanically instead of grepping doc-strings.
//!
//! **The forcing rule (CATALOG REFLECTION + CLOSED-LOOP MASS-SYNTHESIS
//! rule 1):** adding an ecosystem to gen REQUIRES landing its catalog entry
//! here AND its matrix row in `tests/pdc_matrix.rs` in the same commit. The
//! matrix test `matrix_covers_every_catalogued_ecosystem` fails the build
//! when the two drift. The catalog IS the doc; the matrix IS the proof.
//!
//! Tier-honest maturity: cargo = SHIPPED reference (on `main`, conquered);
//! gomod = M1 (per-package dedup proven on jwt-go, encoder on-branch, not
//! yet merged to `main`); npm/pip = Design (adapter scaffolded, but the
//! per-DEPENDENCY content address does not yet exist — npm carries ONE
//! whole-`node_modules` FOD hash, not a per-package address; pip thinner).

use crate::clause::{PdcClause, UnrepTier};
use crate::spec::SourceClass;

/// Maturity gate for a variant's PDC conformance. Mechanical readiness
/// signal (mirrors sui-spec's `Working/M2Typed/…`). Ordered weakest→…→
/// strongest so a histogram + a "no regression" check are trivial.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Maturity {
    /// Adapter scaffolded, but per-DEPENDENCY caching not yet expressible
    /// (no per-node content address, or no substrate interpreter). The PDC
    /// invariant is a design target, not a shipped property.
    Design,
    /// Per-package encoder + interpreter exist and dedup is PROVEN on a
    /// real target, but not yet merged to `main` / not yet the default.
    M1,
    /// Fully shipped on `main`: encoder invariants + substrate interpreter
    /// + real fleet consumers. The reference conquered variant.
    Shipped,
}

impl Maturity {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Maturity::Design => "design",
            Maturity::M1 => "m1",
            Maturity::Shipped => "shipped",
        }
    }
}

/// One variant's per-clause conformance: which clause, and the tier that
/// clause CURRENTLY sits at for THIS variant (never rounded up).
#[derive(Clone, Copy, Debug)]
pub struct ClauseStatus {
    pub clause: PdcClause,
    /// The tier this variant's shipped code actually earns for this clause
    /// today — `None` means the variant does not yet discharge the clause
    /// at all (Design-tier gap).
    pub tier: Option<UnrepTier>,
}

/// A typed point on the PDC lattice — one ecosystem adapter.
#[derive(Clone, Debug)]
pub struct EcosystemVariant {
    /// Ecosystem name (mirrors `Adapter::name()`): `"cargo"`, `"gomod"`, …
    /// The catalog's unique key.
    pub name: &'static str,
    /// The `(def…)` authoring keyword the ecosystem's typed-spec triplet
    /// exposes (TYPED-SPEC TRIPLET + CATALOG REFLECTION field). Globally
    /// unique across the catalog.
    pub adapter_keyword: &'static str,
    /// Overall PDC maturity gate.
    pub maturity: Maturity,
    /// One-line purpose.
    pub purpose: &'static str,
    /// The upstream package-manager surface this variant mirrors.
    pub upstream_mirror: &'static str,
    /// Triplet leg 1 — the typed Rust border crate + its content-address
    /// field name (the clause-1 witness).
    pub border_crate: &'static str,
    pub content_addr_field: &'static str,
    /// Triplet leg 2 — the encoder-side invariant module path.
    pub encoder_invariants: &'static str,
    /// Triplet leg 3 — the substrate Nix interpreter (the dedup fixpoint),
    /// or `""` when it does not yet exist (Design-tier).
    pub nix_interpreter: &'static str,
    /// The `SourceClass` arms this variant admits (clause 5 partition). A
    /// class NOT in this list should be structurally ABSENT from the
    /// variant's kind enum — the checker guards it at value level as the
    /// interim tier.
    pub admitted_classes: &'static [SourceClass],
    /// Per-clause tier status, tier-honest (one entry per `PdcClause`).
    pub clauses: &'static [ClauseStatus],
}

impl EcosystemVariant {
    /// The tier this variant earns for a given clause today (`None` = not
    /// yet discharged).
    #[must_use]
    pub fn tier_for(&self, clause: PdcClause) -> Option<UnrepTier> {
        self.clauses
            .iter()
            .find(|c| c.clause == clause)
            .and_then(|c| c.tier)
    }

    /// Does this variant discharge every PDC clause at least at the
    /// only-mitigated tier? (Shipped/M1 variants do; Design ones do not.)
    #[must_use]
    pub fn discharges_all_clauses(&self) -> bool {
        PdcClause::ALL
            .iter()
            .all(|cl| self.tier_for(*cl).is_some())
    }
}

// ── clause-status shorthands ─────────────────────────────────────────────
const fn cs(clause: PdcClause, tier: UnrepTier) -> ClauseStatus {
    ClauseStatus { clause, tier: Some(tier) }
}
const fn gap(clause: PdcClause) -> ClauseStatus {
    ClauseStatus { clause, tier: None }
}

use PdcClause::*;
use SourceClass::*;
use UnrepTier::*;

/// cargo — the SHIPPED reference. gen-cargo v10 + rust/lockfile-builder.nix.
/// Every clause discharged; content-address is a runtime `Violation` today
/// (`RegistryWithoutSha256`) — only-mitigated until the border adopts
/// `ContentAddr` (the FIXABLE the seal names). Kind⟺source is the
/// Target/Host + proc_macro split (truly-unrep for the placement arm).
const CARGO: EcosystemVariant = EcosystemVariant {
    name: "cargo",
    adapter_keyword: "defcargo-build-spec",
    maturity: Maturity::Shipped,
    purpose: "Rust workspace → per-crate buildRustCrate dedup graph",
    upstream_mirror: "cargo / crates.io + Cargo.lock",
    border_crate: "gen-cargo::build_spec::BuildSpec",
    content_addr_field: "CrateSource::Registry.sha256 / cargo_lock_sha256",
    encoder_invariants: "gen-cargo::invariants (11 rules)",
    nix_interpreter: "substrate/lib/build/rust/lockfile-builder.nix (lib.fix)",
    admitted_classes: &[WorkspaceMember, Registry],
    clauses: &[
        cs(ContentAddressed, OnlyMitigated),
        cs(EdgeResolvable, OnlyMitigated),
        cs(DedupByAddress, CeilingC1),
        cs(BoundaryPrecise, CeilingC1),
        cs(KindSourceDisjoint, TrulyUnrep),
        cs(FreshnessGated, OnlyMitigated),
    ],
};

/// gomod — M1. gen-gomod per-package encoder + go/package-graph.nix.
/// Per-package cross-binary dedup PROVEN on jwt-go. The literal per-dep
/// caching invariant is named: Go-I8 `NodeMissingSourceHash`. Kind⟺source
/// is truly-unrep for the cgo/asm/tool exclusion (Go-I12: no arm exists).
/// M1 because the encoder lives on branches, not yet merged to `main`.
const GOMOD: EcosystemVariant = EcosystemVariant {
    name: "gomod",
    adapter_keyword: "defgomod-build-spec",
    maturity: Maturity::M1,
    purpose: "Go module → per-package archive dedup graph (compiled once, linked into every binary)",
    upstream_mirror: "go mod / the Go module proxy + go.sum",
    border_crate: "gen-gomod::build_spec::BuildSpec",
    content_addr_field: "PdcNode.source_hash (Go-I8)",
    encoder_invariants: "gen-gomod::invariants (Go-I1/3/8/9/10/11/12)",
    nix_interpreter: "substrate/lib/build/go/package-graph.nix (lib.fix + ferrite tree)",
    admitted_classes: &[WorkspaceMember, Vendored, Registry, Std],
    clauses: &[
        cs(ContentAddressed, OnlyMitigated),
        cs(EdgeResolvable, OnlyMitigated),
        cs(DedupByAddress, CeilingC1),
        cs(BoundaryPrecise, CeilingC1),
        // Go-I12: cgo/asm/tool arms are ABSENT from PackageKind → truly-unrep.
        cs(KindSourceDisjoint, TrulyUnrep),
        cs(FreshnessGated, OnlyMitigated),
    ],
};

/// npm — Design. gen-npm adapter scaffolded, BUT the per-DEPENDENCY content
/// address does not yet exist: `npmDepsHash` is ONE FOD hash of the whole
/// `node_modules` tar, not a per-package address. So PDC clause 1 holds
/// only at WHOLE-TREE granularity — editing one dependency re-derives the
/// entire tree, not just that dependency + its dependents (clause 4 fails).
/// No substrate npm interpreter. The per-package split is the design work.
const NPM: EcosystemVariant = EcosystemVariant {
    name: "npm",
    adapter_keyword: "defnpm-build-spec",
    maturity: Maturity::Design,
    purpose: "npm package → buildNpmPackage (per-package dedup is the design target)",
    upstream_mirror: "npm / the npm registry + package-lock.json",
    border_crate: "gen-npm::build_spec::BuildSpec",
    content_addr_field: "npmDepsHash (WHOLE-TREE FOD, not per-package — the gap)",
    encoder_invariants: "gen-npm::invariants (schema + pname/version only)",
    nix_interpreter: "",
    admitted_classes: &[WorkspaceMember, Registry],
    clauses: &[
        // Whole-tree hash only ⇒ per-DEPENDENCY clause 1 is not discharged.
        gap(ContentAddressed),
        cs(EdgeResolvable, OnlyMitigated),
        gap(DedupByAddress),
        gap(BoundaryPrecise),
        gap(KindSourceDisjoint),
        cs(FreshnessGated, OnlyMitigated),
    ],
};

/// pip / pyproject — Design. gen-pip adapter scaffolded (thin invariants).
/// No per-wheel content address on the border, no substrate python
/// interpreter. Every PDC clause beyond freshness is a design target.
const PIP: EcosystemVariant = EcosystemVariant {
    name: "pip",
    adapter_keyword: "defpip-build-spec",
    maturity: Maturity::Design,
    purpose: "Python wheel/sdist → buildPythonPackage (per-wheel dedup is the design target)",
    upstream_mirror: "pip / PyPI + requirements/pyproject",
    border_crate: "gen-pip::build_spec::BuildSpec",
    content_addr_field: "(none yet — the gap)",
    encoder_invariants: "gen-pip::invariants (schema only)",
    nix_interpreter: "",
    admitted_classes: &[WorkspaceMember, Registry],
    clauses: &[
        gap(ContentAddressed),
        gap(EdgeResolvable),
        gap(DedupByAddress),
        gap(BoundaryPrecise),
        gap(KindSourceDisjoint),
        cs(FreshnessGated, OnlyMitigated),
    ],
};

/// The full PDC variant catalog. Order: strongest maturity first, then
/// alphabetical — stable for diffs. Adding an ecosystem = adding a `const`
/// + one entry here + one matrix row (same commit; the matrix enforces it).
pub const CATALOG: &[&EcosystemVariant] = &[&CARGO, &GOMOD, &NPM, &PIP];

/// Look up a variant by name.
#[must_use]
pub fn variant(name: &str) -> Option<&'static EcosystemVariant> {
    CATALOG.iter().copied().find(|v| v.name == name)
}

/// Maturity histogram — `(design, m1, shipped)`. Sums to `CATALOG.len()`
/// (partition-complete; the catalog test asserts it).
#[must_use]
pub fn maturity_histogram() -> (usize, usize, usize) {
    let mut h = (0, 0, 0);
    for v in CATALOG {
        match v.maturity {
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
    fn every_variant_name_is_unique() {
        // CATALOG REFLECTION: no two catalog entries share a name.
        let mut names: Vec<&str> = CATALOG.iter().map(|v| v.name).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(before, names.len(), "duplicate ecosystem name in CATALOG");
    }

    #[test]
    fn every_adapter_keyword_is_globally_unique() {
        // CATALOG REFLECTION: authoring keywords never collide.
        let mut kws: Vec<&str> = CATALOG.iter().map(|v| v.adapter_keyword).collect();
        kws.sort_unstable();
        let before = kws.len();
        kws.dedup();
        assert_eq!(before, kws.len(), "duplicate adapter_keyword in CATALOG");
    }

    #[test]
    fn every_variant_declares_every_clause_exactly_once() {
        // CATALOG REFLECTION: the per-clause status is a complete partition
        // of the six clauses (no missing clause, no duplicate).
        for v in CATALOG {
            for clause in PdcClause::ALL {
                let n = v.clauses.iter().filter(|c| c.clause == clause).count();
                assert_eq!(
                    n, 1,
                    "variant {} must declare clause {clause:?} exactly once (found {n})",
                    v.name
                );
            }
            assert_eq!(
                v.clauses.len(),
                PdcClause::ALL.len(),
                "variant {} declares extra/duplicate clause rows",
                v.name
            );
        }
    }

    #[test]
    fn maturity_histogram_partitions_the_catalog() {
        let (d, m1, s) = maturity_histogram();
        assert_eq!(d + m1 + s, CATALOG.len(), "maturity histogram must sum to catalog size");
        // Tier-honest snapshot of the CURRENT fleet state (2026-07):
        // cargo shipped, gomod M1, npm+pip design. Update deliberately when
        // a variant advances — this line IS the ledger.
        assert_eq!((d, m1, s), (2, 1, 1), "PDC maturity ledger drifted — update deliberately");
    }

    #[test]
    fn no_variant_rounds_a_clause_tier_up_past_its_achievable_ceiling() {
        // A variant's per-clause tier must never be STRONGER than the tier
        // that clause can achieve at its destination — rounding up is the
        // cardinal honesty sin. (Weaker is fine: a shipped variant may sit
        // below the achievable tier; that is the burn-down backlog.)
        use crate::clause::UnrepTier::*;
        fn rank(t: crate::clause::UnrepTier) -> u8 {
            match t {
                OnlyMitigated => 0,
                CeilingC1 => 1, // a ceiling is "as good as it gets" for its clause
                ParseTimeRejected => 2,
                TrulyUnrepLibraryParseWire => 3,
                TrulyUnrep => 4,
            }
        }
        for v in CATALOG {
            for cs in v.clauses {
                if let Some(t) = cs.tier {
                    let ach = cs.clause.achievable_tier();
                    // C1-ceiling clauses: the variant may only claim exactly
                    // the ceiling (CeilingC1) or below (OnlyMitigated).
                    assert!(
                        rank(t) <= rank(ach),
                        "variant {} rounds clause {:?} UP: claims {:?}, achievable {:?}",
                        v.name, cs.clause, t, ach
                    );
                }
            }
        }
    }

    #[test]
    fn shipped_and_m1_variants_discharge_every_clause_design_ones_do_not() {
        for v in CATALOG {
            match v.maturity {
                Maturity::Shipped | Maturity::M1 => assert!(
                    v.discharges_all_clauses(),
                    "{} is {} but leaves a clause undischarged",
                    v.name, v.maturity.as_str()
                ),
                Maturity::Design => assert!(
                    !v.discharges_all_clauses(),
                    "{} is design yet discharges every clause — promote its maturity",
                    v.name
                ),
            }
        }
    }
}
