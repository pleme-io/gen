//! Smoke test: the scaffolded gen-helm crate's trait surface is
//! reachable from outside the crate. Catches regressions where
//! `gen scaffold-adapter` would emit a crate that doesn't actually
//! implement the universal trait surface (Spec / QuirkRegistry /
//! Invariants / Adapter).
//!
//! When a real adapter author fills in `Adapter::build`, these
//! tests stay green — the scaffold's job is to make the trait
//! surface compile from day zero.

use gen_helm::adapter::HelmAdapter;
use gen_helm::invariants::HelmInvariants;
use gen_helm::quirks::HelmQuirks;
use gen_types::{Adapter, Invariants, QuirkRegistry};

#[test]
fn adapter_name_matches_ecosystem() {
    let a = HelmAdapter;
    assert_eq!(a.name(), "helm");
    assert_eq!(a.manifest_files(), &["Chart.yaml"]);
}

#[test]
fn empty_quirk_registry_is_callable() {
    // Default scaffold ships with an empty registry — the trait
    // surface should still work; for_package and registered_names
    // both return empty results.
    let names = <HelmQuirks as QuirkRegistry>::registered_names();
    assert!(names.is_empty());
    let q = <HelmQuirks as QuirkRegistry>::for_package("anything");
    assert!(q.is_empty());
}

#[test]
fn adapter_exposes_quirks_via_default_envelope() {
    // The Adapter::quirks_registry trait method should return an
    // envelope of the adapter's typed registry. Empty for the
    // freshly-scaffolded adapter.
    let a = HelmAdapter;
    let entries = a.quirks_registry();
    assert!(entries.is_empty());
}

#[test]
fn invariants_run_clean_against_minimal_spec() {
    use gen_helm::build_spec::BuildSpec;
    use indexmap::IndexMap;
    let spec = BuildSpec {
        version: gen_helm::build_spec::SCHEMA_VERSION,
        packages: IndexMap::new(),
        root_package: String::new(),
        workspace_members: vec![],
    };
    let violations = <HelmInvariants as Invariants>::check(&spec);
    assert!(violations.is_empty(), "minimal spec violated: {violations:?}");
}
