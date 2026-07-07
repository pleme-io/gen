//! The UNIVERSAL PDC checker — one law, applied to any variant's `PdcSpec`.
//!
//! This collapses the six per-ecosystem `check()` fns (gen-cargo's 11
//! rules, gen-gomod's Go-I1/3/8/9/10/11/12, …) to ONE definition of the
//! ecosystem-agnostic clauses. A variant's own `check()` still adds its
//! ecosystem-specific rows; the clauses HERE are the shared floor every
//! variant must pass — the `check_validity()` of the PDC lattice
//! (`architecture_lattice.rs`'s model, applied to per-dependency caching).
//!
//! `/algorithmic-prowess-seal` — the algorithms, best-fit, no ML:
//! - **clause 2 (edge-resolvable):** a set-membership walk over the node map.
//! - **clause 1 (content-addressed) + clause 3/4 (dedup / boundary):** a
//!   **lazy memoized topological fixpoint** over the dependency DAG — each
//!   node's Merkle address is computed **exactly once** and reused by every
//!   dependent (the pure-Rust analogue of substrate's `lib.fix (self:
//!   mapAttrs …)`). The fixpoint proves clause 3 constructively: a
//!   `HashMap` memo IS the "computed once" witness, and a re-request of a
//!   memoized key is a hit, never a recomputation. Cycle detection is a
//!   greyed-node DFS (Tarjan-style colouring) so a cyclic spec is a typed
//!   violation, not a stack overflow.
//! - **clause 5 (kind ⟺ source):** the variant's admitted `SourceClass`
//!   set (from the catalog) is the partition; a node outside it is a
//!   violation. The DESTINATION for this clause is an absent enum arm
//!   (truly-unrep) — the checker is the interim value-level guard.
//! - **clause 6 (freshness):** an integer comparison.

use std::collections::HashMap;

use crate::content_addr::ContentAddr;
use crate::spec::{PdcSpec, SourceClass};

/// A universal PDC violation — the shared law's failure modes, named once.
/// A variant's ecosystem-specific violations are a *separate* enum it adds;
/// these are the clauses every variant shares.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PdcViolation {
    /// Clause 1: a node whose class requires an address carries none.
    MissingContentAddress { key: String, class: SourceClass },
    /// Clause 1 (Merkle agreement): the declared address is not
    /// `H(source ∪ dep-addrs)` — the address is a free token, not a true
    /// content address, so a source edit would NOT re-derive it and the
    /// cache key would be stale-yet-hit. The strongest clause-1 check.
    AddressNotMerkleConsistent { key: String },
    /// Clause 2: an edge names a node absent from the spec.
    UnresolvedEdge { from: String, missing: String },
    /// Clause 3/4: the dependency graph has a cycle — no topological order,
    /// so the "computed once" fixpoint cannot terminate.
    DependencyCycle { key: String },
    /// Clause 5: a node's source class is outside the variant's admitted
    /// set (the variant should not be able to build it at all).
    KindNotAdmitted { key: String, class: SourceClass },
    /// Clause 5: a declared workspace member is absent or is not a member
    /// node.
    MemberNotAMember { key: String },
    /// Clause 6: the spec is below the variant's current schema version.
    StaleSchema { found: u32, expected: u32 },
}

/// Outcome of a PDC check: the violations (empty = well-formed) plus the
/// dedup witness — how many DISTINCT node addresses the fixpoint computed
/// vs. how many total dependency requests were served. `requests > computed`
/// is the *measured* dedup: the shared-node-built-once win, quantified.
#[derive(Clone, Debug)]
pub struct PdcCheckOutcome {
    /// Every clause violation, deterministic order.
    pub violations: Vec<PdcViolation>,
    /// Distinct node addresses the fixpoint computed (each node once).
    pub nodes_computed: usize,
    /// Total dependency-address requests served from the memo. When this
    /// exceeds `nodes_computed`, a node was reused by ≥2 dependents — the
    /// dedup-by-address win, observed. (0 for a spec with no shared deps.)
    pub addr_requests_served: usize,
}

impl PdcCheckOutcome {
    /// Well-formed against the shared PDC law?
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.violations.is_empty()
    }

    /// Was any node reused by more than one dependent (dedup observed)?
    #[must_use]
    pub fn observed_dedup(&self) -> bool {
        self.addr_requests_served > self.nodes_computed
    }
}

/// Run the universal PDC clauses over one variant's projected spec.
///
/// `admitted` is the variant's admitted `SourceClass` set (from the
/// catalog) — the clause-5 partition. Pure, deterministic, no I/O.
#[must_use]
pub fn check(spec: &PdcSpec, admitted: &[SourceClass]) -> PdcCheckOutcome {
    let mut violations = Vec::new();

    // ── clause 6 — freshness ────────────────────────────────────────────
    if spec.schema_version < spec.current_schema_version {
        violations.push(PdcViolation::StaleSchema {
            found: spec.schema_version,
            expected: spec.current_schema_version,
        });
    }

    // ── clause 2 — edge-resolvable + clause 5 — kind admitted ───────────
    for (key, node) in &spec.nodes {
        if !admitted.contains(&node.class) {
            violations.push(PdcViolation::KindNotAdmitted {
                key: key.clone(),
                class: node.class,
            });
        }
        if node.class.requires_address() && node.addr.is_none() {
            violations.push(PdcViolation::MissingContentAddress {
                key: key.clone(),
                class: node.class,
            });
        }
        for dep in &node.deps {
            if !spec.nodes.contains_key(dep) {
                violations.push(PdcViolation::UnresolvedEdge {
                    from: key.clone(),
                    missing: dep.clone(),
                });
            }
        }
    }

    // ── clause 5 — workspace member correspondence ──────────────────────
    for m in &spec.members {
        match spec.nodes.get(m) {
            Some(n) if n.is_member => {}
            _ => violations.push(PdcViolation::MemberNotAMember { key: m.clone() }),
        }
    }

    // ── clause 1 + 3 + 4 — the lazy memoized dedup fixpoint ─────────────
    // Only run when edges resolve (a dangling edge would make the fixpoint
    // ill-defined); the clause-2 rows above already report those.
    let edges_resolve = !violations
        .iter()
        .any(|v| matches!(v, PdcViolation::UnresolvedEdge { .. }));
    let (nodes_computed, addr_requests_served) = if edges_resolve {
        run_fixpoint(spec, &mut violations)
    } else {
        (0, 0)
    };

    PdcCheckOutcome {
        violations,
        nodes_computed,
        addr_requests_served,
    }
}

/// The lazy memoized topological fixpoint. Returns `(distinct nodes
/// computed, total dep-address requests served)`.
///
/// This is the pure-Rust analogue of `lib.fix (self: mapAttrs …)`: `memo`
/// is `self`, `compute` fills it once per key. Every dependent of a shared
/// node hits the memo — the "shared internal package compiled once, linked
/// into every binary" property, made a runtime witness (clause 3), and the
/// Merkle-agreement check (clause 1) that the declared address genuinely
/// equals `H(source ∪ dep-addrs)`.
fn run_fixpoint(spec: &PdcSpec, violations: &mut Vec<PdcViolation>) -> (usize, usize) {
    // Colours for cycle detection (Tarjan-style): White = unseen,
    // Grey = on the current DFS stack, Black = done.
    #[derive(Clone, Copy, PartialEq)]
    enum Colour {
        Grey,
        Black,
    }
    let mut colour: HashMap<&str, Colour> = HashMap::new();
    // The memo — each key's computed Merkle address, filled exactly once.
    let mut memo: HashMap<&str, ContentAddr> = HashMap::new();
    // Instrumentation: how many times we SERVED an address from the memo
    // to a dependent (vs. computed a fresh one). requests > computed ⇒ dedup.
    let mut requests: usize = 0;

    // Recursive post-order compute. Returns the node's Merkle address (or
    // None for a Std node / a node in a cycle — the caller folds None deps
    // out, and the cycle is already recorded).
    fn compute<'a>(
        key: &'a str,
        spec: &'a PdcSpec,
        colour: &mut HashMap<&'a str, Colour>,
        memo: &mut HashMap<&'a str, ContentAddr>,
        requests: &mut usize,
        violations: &mut Vec<PdcViolation>,
    ) -> Option<ContentAddr> {
        if let Some(a) = memo.get(key) {
            // Memo hit — the dedup witness. A dependent reused a node that
            // was already computed. This is clause 3 observed.
            *requests += 1;
            return Some(a.clone());
        }
        match colour.get(key) {
            Some(Colour::Grey) => {
                // Back-edge → cycle. Record once; unwind with None.
                violations.push(PdcViolation::DependencyCycle { key: key.to_string() });
                return None;
            }
            // Black-but-not-memoized ⇒ an address-less node (Std) already
            // fully processed on a prior root. It contributes no address to
            // the Merkle fold and needs no recompute — return None. (An
            // address-BEARING node that is Black is always in the memo and
            // was handled by the memo-hit branch above, so it never reaches
            // here.)
            Some(Colour::Black) => return None,
            None => {}
        }
        colour.insert(key, Colour::Grey);

        let node = spec.nodes.get(key).expect("edges resolve (checked) ⇒ key present");
        // Compute children first (post-order) so their addresses fold in.
        let mut child_addrs: Vec<ContentAddr> = Vec::new();
        for dep in &node.deps {
            if let Some(a) = compute(dep, spec, colour, memo, requests, violations) {
                child_addrs.push(a);
            }
        }

        colour.insert(key, Colour::Black);

        // Std nodes carry no per-node address — they resolve to the shared
        // std tree. Nothing to memo, nothing to Merkle-check (clause 1
        // exempts Std).
        if !node.class.requires_address() {
            return None;
        }

        // clause 1 — Merkle agreement. The declared address MUST equal
        // H(source ∪ dep-addrs). If it doesn't, the address is a free token
        // that a source edit wouldn't re-derive — a stale-yet-hit cache key.
        let recomputed = ContentAddr::merkle(&node.source, child_addrs.iter().collect());
        if let Some(declared) = &node.addr {
            if declared != &recomputed {
                violations.push(PdcViolation::AddressNotMerkleConsistent {
                    key: key.to_string(),
                });
            }
        }
        // (MissingContentAddress for addr==None is reported in the main
        // pass; here we memo the RECOMPUTED address so the fixpoint is
        // total even when the declared one was absent/inconsistent — the
        // dedup structure is what we're proving, independent of the
        // declared-value bug.)
        memo.insert(key, recomputed.clone());
        Some(recomputed)
    }

    // Drive from every node so disconnected components + every root are
    // covered. Roots first (members) for a natural post-order, but the
    // memo makes order irrelevant to the result.
    let order: Vec<&str> = spec
        .members
        .iter()
        .map(String::as_str)
        .chain(spec.nodes.keys().map(String::as_str))
        .collect();
    for key in order {
        if spec.nodes.contains_key(key) {
            let _ = compute(key, spec, &mut colour, &mut memo, &mut requests, violations);
        }
    }

    (memo.len(), requests)
}
