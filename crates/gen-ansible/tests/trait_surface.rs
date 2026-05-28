//! Trait-surface smoke tests for the scaffolded gen-ansible adapter.
//! Auto-emitted by `gen scaffold-adapter` — the four universal trait
//! surfaces (Adapter / Spec / QuirkRegistry / Invariants) must compile
//! + behave consistently from the day the crate is scaffolded.

use gen_ansible::adapter::AnsibleAdapter;
use gen_ansible::invariants::AnsibleInvariants;
use gen_ansible::quirks::AnsibleQuirks;
use gen_types::{Adapter, Invariants, QuirkRegistry};

#[test]
fn adapter_name_matches_ecosystem() {
    let a = AnsibleAdapter;
    assert_eq!(a.name(), "ansible");
    assert_eq!(a.manifest_files(), &["galaxy.yml"]);
}

#[test]
fn empty_quirk_registry_is_callable() {
    let names = <AnsibleQuirks as QuirkRegistry>::registered_names();
    assert!(names.is_empty());
    let q = <AnsibleQuirks as QuirkRegistry>::for_package("anything");
    assert!(q.is_empty());
}

#[test]
fn adapter_exposes_quirks_via_default_envelope() {
    let a = AnsibleAdapter;
    let entries = a.quirks_registry();
    assert!(entries.is_empty());
}

#[test]
fn invariants_run_clean_against_minimal_spec() {
    use gen_ansible::build_spec::BuildSpec;
    use indexmap::IndexMap;
    let spec = BuildSpec {
        version: gen_ansible::build_spec::SCHEMA_VERSION,
        packages: IndexMap::new(),
        root_package: String::new(),
        workspace_members: vec![],
    };
    let violations = <AnsibleInvariants as Invariants>::check(&spec);
    assert!(violations.is_empty(), "minimal spec violated: {violations:?}");
}
