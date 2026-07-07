//! Synthetic per-ecosystem `PdcSpec` fixtures the verification matrix
//! exercises.
//!
//! gen-pdc depends ONLY on `gen-types` (deliberately — see the crate doc),
//! so it cannot call a live adapter's real `project(&BuildSpec)`. Instead
//! each fixture mirrors an ecosystem's real node/edge SHAPE faithfully
//! enough to exercise the PDC law:
//!
//! - a shared dependency built ONCE and reused by ≥2 consumers (the
//!   invariant the matrix asserts — [`crate::check::PdcCheckOutcome::observed_dedup`]);
//! - the ecosystem's real source classes (gomod carries `Std`, cargo does
//!   not model it as a node, npm carries a whole-tree hash);
//! - Merkle-consistent declared addresses so clause 1's agreement check
//!   passes for the shipped/M1 rows.
//!
//! The DESIGN rows (npm/pip) get a fixture that faithfully shows WHY they
//! fail the per-dependency invariant (npm: one shared node has no per-node
//! address → the whole-tree-hash gap), so the matrix records the gap
//! rather than pretending conformance.

use indexmap::IndexMap;

use crate::content_addr::ContentAddr;
use crate::spec::{PdcNode, PdcSpec, SourceClass};

/// Build a Merkle-consistent node: its declared address is exactly
/// `H(source ∪ dep-addrs)`, so clause 1's agreement check passes.
fn node(
    class: SourceClass,
    source: &[u8],
    dep_addrs: &[&ContentAddr],
    deps: Vec<String>,
    is_member: bool,
) -> PdcNode {
    let addr = if class.requires_address() {
        Some(ContentAddr::merkle(source, dep_addrs.to_vec()))
    } else {
        None
    };
    PdcNode {
        class,
        addr,
        source: source.to_vec(),
        deps,
        is_member,
    }
}

/// A CONFORMANT fixture with a shared dependency (the "compiled once,
/// linked into every binary" win). Two members (`bin-a`, `bin-b`) both
/// depend on a shared registry crate (`shared`), which depends on a leaf
/// (`leaf`). The dedup fixpoint computes `shared` + `leaf` once and serves
/// them to both members — `observed_dedup()` is true.
///
/// `classes.contains(Std)` toggles a std node (gomod-shaped) vs. not
/// (cargo-shaped). This is the SHIPPED/M1 fixture shape.
#[must_use]
pub fn conformant_shared_dep(with_std: bool) -> PdcSpec {
    let mut nodes = IndexMap::new();

    let leaf = node(SourceClass::Registry, b"leaf-src", &[], vec![], false);
    let leaf_addr = leaf.addr.clone().unwrap();
    nodes.insert("leaf".to_string(), leaf);

    // `shared` depends on `leaf` (+ optionally a std node, gomod shape).
    // Std contributes no address to the Merkle fold (it has none), so the
    // dep-address list is just `leaf`'s address either way.
    let mut shared_deps = vec!["leaf".to_string()];
    if with_std {
        let std = node(SourceClass::Std, b"", &[], vec![], false);
        nodes.insert("std/fmt".to_string(), std);
        shared_deps.push("std/fmt".to_string());
    }
    let shared = node(
        SourceClass::Registry,
        b"shared-src",
        &[&leaf_addr],
        shared_deps,
        false,
    );
    let shared_addr = shared.addr.clone().unwrap();
    nodes.insert("shared".to_string(), shared);

    let bin_a = node(
        SourceClass::WorkspaceMember,
        b"bin-a-src",
        &[&shared_addr],
        vec!["shared".to_string()],
        true,
    );
    nodes.insert("bin-a".to_string(), bin_a);
    let bin_b = node(
        SourceClass::WorkspaceMember,
        b"bin-b-src",
        &[&shared_addr],
        vec!["shared".to_string()],
        true,
    );
    nodes.insert("bin-b".to_string(), bin_b);

    PdcSpec {
        schema_version: 10,
        current_schema_version: 10,
        nodes,
        members: vec!["bin-a".to_string(), "bin-b".to_string()],
    }
}

/// The cargo-shaped conformant fixture (no std node modelled).
#[must_use]
pub fn cargo_fixture() -> PdcSpec {
    conformant_shared_dep(false)
}

/// The gomod-shaped conformant fixture (carries a `Std` node — Go-I10).
#[must_use]
pub fn gomod_fixture() -> PdcSpec {
    conformant_shared_dep(true)
}

/// The npm-shaped DESIGN fixture. Same graph, but the shared dependency
/// carries NO per-node address (npm's `npmDepsHash` is a whole-tree FOD,
/// not a per-package address). This faithfully reproduces the per-DEPENDENCY
/// gap: the checker reports `MissingContentAddress` for `shared`, so the
/// matrix records npm as NOT discharging clause 1 — the honest design-tier
/// state, not a pretend pass.
#[must_use]
pub fn npm_design_fixture() -> PdcSpec {
    let mut spec = conformant_shared_dep(false);
    // Strip the shared node's per-node address → the whole-tree-hash gap.
    if let Some(n) = spec.nodes.get_mut("shared") {
        n.addr = None;
    }
    if let Some(n) = spec.nodes.get_mut("leaf") {
        n.addr = None;
    }
    spec
}

/// The pip-shaped DESIGN fixture. No per-node addresses anywhere and no
/// resolved edges modelled (pip's border carries neither yet). Every clause
/// beyond freshness is a gap.
#[must_use]
pub fn pip_design_fixture() -> PdcSpec {
    let mut nodes = IndexMap::new();
    // A single member with a dangling edge + no address — the thinnest
    // possible shape, reflecting that pip's border discharges nothing yet.
    nodes.insert(
        "app".to_string(),
        PdcNode {
            class: SourceClass::WorkspaceMember,
            addr: None,
            source: b"app".to_vec(),
            deps: vec!["requests".to_string()], // dangling — not in nodes
            is_member: true,
        },
    );
    PdcSpec {
        schema_version: 1,
        current_schema_version: 1,
        nodes,
        members: vec!["app".to_string()],
    }
}
