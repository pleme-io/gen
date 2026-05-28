//! Trait-surface smoke tests for the scaffolded gen-swift adapter.
//! Auto-emitted by `gen scaffold-adapter` — the four universal trait
//! surfaces (Adapter / Spec / QuirkRegistry / Invariants) must compile
//! + behave consistently from the day the crate is scaffolded.

use gen_swift::adapter::SwiftAdapter;
use gen_swift::invariants::SwiftInvariants;
use gen_swift::quirks::SwiftQuirks;
use gen_types::{Adapter, Invariants, QuirkRegistry};

#[test]
fn adapter_name_matches_ecosystem() {
    let a = SwiftAdapter;
    assert_eq!(a.name(), "swift");
    assert_eq!(a.manifest_files(), &["Package.swift"]);
}

#[test]
fn empty_quirk_registry_is_callable() {
    let names = <SwiftQuirks as QuirkRegistry>::registered_names();
    assert!(names.is_empty());
    let q = <SwiftQuirks as QuirkRegistry>::for_package("anything");
    assert!(q.is_empty());
}

#[test]
fn adapter_exposes_quirks_via_default_envelope() {
    let a = SwiftAdapter;
    let entries = a.quirks_registry();
    assert!(entries.is_empty());
}

#[test]
fn invariants_run_clean_against_minimal_spec() {
    use gen_swift::build_spec::BuildSpec;
    use indexmap::IndexMap;
    let spec = BuildSpec {
        version: gen_swift::build_spec::SCHEMA_VERSION,
        packages: IndexMap::new(),
        root_package: String::new(),
        workspace_members: vec![],
    };
    let violations = <SwiftInvariants as Invariants>::check(&spec);
    assert!(violations.is_empty(), "minimal spec violated: {violations:?}");
}
