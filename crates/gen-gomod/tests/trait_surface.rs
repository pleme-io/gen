//! Trait-surface smoke tests for the scaffolded gen-gomod adapter.
//! Auto-emitted by `gen scaffold-adapter` — the four universal trait
//! surfaces (Adapter / Spec / QuirkRegistry / Invariants) must compile
//! + behave consistently from the day the crate is scaffolded.

use gen_gomod::adapter::GomodAdapter;
use gen_gomod::invariants::GomodInvariants;
use gen_gomod::quirks::GomodQuirks;
use gen_types::{Adapter, Invariants, QuirkRegistry};

#[test]
fn adapter_name_matches_ecosystem() {
    let a = GomodAdapter;
    assert_eq!(a.name(), "gomod");
    assert_eq!(a.manifest_files(), &["go.mod"]);
}

#[test]
fn empty_quirk_registry_is_callable() {
    let names = <GomodQuirks as QuirkRegistry>::registered_names();
    assert!(names.is_empty());
    let q = <GomodQuirks as QuirkRegistry>::for_package("anything");
    assert!(q.is_empty());
}

#[test]
fn adapter_exposes_quirks_via_default_envelope() {
    let a = GomodAdapter;
    let entries = a.quirks_registry();
    assert!(entries.is_empty());
}

#[test]
fn invariants_run_clean_against_minimal_spec() {
    use gen_gomod::build_spec::{BuildSpec, DepMode, ModuleSpec, Renderer, SCHEMA_VERSION};
    use std::collections::BTreeMap;
    let spec = BuildSpec {
        version: SCHEMA_VERSION,
        renderer: Renderer::Incremental,
        module: ModuleSpec {
            module_path: "example.com/x".into(),
            go_version: "1.25".into(),
            toolchain: None,
            has_external_deps: false,
            dep_mode: DepMode::Vendored,
            vendor_hash: None,
        },
        packages: BTreeMap::new(),
        root_package: String::new(),
        workspace_members: vec![],
        target_resolves: None,
        go_sum_sha256: None,
    };
    let violations = <GomodInvariants as Invariants>::check(&spec);
    assert!(violations.is_empty(), "minimal spec violated: {violations:?}");
}
