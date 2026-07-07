//! `PdcSpec` — the ecosystem-agnostic projection every variant reduces to.
//!
//! A variant's typed `BuildSpec` (gen-cargo's crate graph, gen-gomod's
//! package graph, …) carries far more than PDC needs. The PDC invariant is
//! a law over exactly one shape: **a set of buildable nodes, each with a
//! source-class + a content address, wired by dependency edges**. This
//! module is that shape — the normalized view the universal checker
//! ([`crate::check`]) operates over.
//!
//! Every ecosystem variant supplies a `fn project(&BuildSpec) -> PdcSpec`.
//! For the shipped/M1 rows a real projection would read the adapter's
//! typed spec; here (gen-pdc depends only on `gen-types`) the projections
//! are the synthetic fixtures in [`crate::fixture`], which mirror each
//! ecosystem's real node/edge shape faithfully enough to exercise the law.

use indexmap::IndexMap;

use crate::content_addr::ContentAddr;

/// The disjoint source classes a node can carry. This is the universal
/// union of every ecosystem's source kinds; a *variant's* kind enum is a
/// subset with the arms it cannot build ABSENT (PDC clause 5). The variant
/// declares which arms it admits via [`crate::catalog::EcosystemVariant`];
/// a node whose class is outside the variant's admitted set is a clause-5
/// violation caught by the checker.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SourceClass {
    /// A first-party workspace member (path source). Carries a source hash.
    WorkspaceMember,
    /// A vendored / path-relative source inside the workspace tree.
    Vendored,
    /// A registry / module-proxy source (crates.io, the Go module proxy,
    /// the npm registry, PyPI). Carries a content address.
    Registry,
    /// A language standard library node — the ONE class that legitimately
    /// carries NO per-node address (it resolves to a shared std tree).
    /// (gomod `Std`.)
    Std,
}

impl SourceClass {
    /// Whether a node of this class must carry a non-empty content address
    /// (PDC clause 1). Every class except `Std` does. This is the
    /// kind⟺content-address partition.
    #[must_use]
    pub fn requires_address(self) -> bool {
        !matches!(self, SourceClass::Std)
    }
}

/// One buildable node in the normalized PDC graph.
#[derive(Clone, Debug)]
pub struct PdcNode {
    /// The node's disjoint source class (clause 5).
    pub class: SourceClass,
    /// The per-dependency content address (clause 1). `None` iff `Std`
    /// (the one class that legitimately carries no address). For every
    /// other class this being `Some` is the sealed clause-1 witness — the
    /// type ([`ContentAddr`]) already made "present but empty"
    /// unrepresentable, so the checker only tests presence-when-required.
    pub addr: Option<ContentAddr>,
    /// Raw source bytes, for recomputing `addr` in the dedup fixpoint.
    /// (In a real projection this is a digest handle, not the bytes; the
    /// fixtures carry small byte strings.)
    pub source: Vec<u8>,
    /// Direct dependency edges — keys into the same `PdcSpec.nodes` map.
    pub deps: Vec<String>,
    /// Is this node a workspace root / linkable binary target? (clause 5
    /// workspace_member ⟺ Main/root correspondence.)
    pub is_member: bool,
}

/// The normalized per-dependency-caching spec. Ecosystem-agnostic.
#[derive(Clone, Debug)]
pub struct PdcSpec {
    /// Spec schema version, compared against `current_schema_version`
    /// (clause 6). A variant projects its own `schema_version()` here.
    pub schema_version: u32,
    /// The schema version the emitting variant is currently at. A spec
    /// below this is stale (clause 6).
    pub current_schema_version: u32,
    /// Every buildable node, keyed by its spec key. `IndexMap` for stable
    /// diff order (mirrors the adapters' `IndexMap`/`BTreeMap` crates).
    pub nodes: IndexMap<String, PdcNode>,
    /// Workspace member / root keys — must resolve to a `is_member` node
    /// (clause 5).
    pub members: Vec<String>,
}
