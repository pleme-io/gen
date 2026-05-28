use gen_ansible::build_spec::{BuildSpec, PackageArgs, PackageSpec, SCHEMA_VERSION};
use gen_ansible::invariants::AnsibleInvariants;
use gen_ansible::quirks::{AnsibleQuirk, AnsibleQuirks};
use gen_types::{Adapter, Invariants, QuirkRegistry, Spec};
use indexmap::IndexMap;

#[test]
fn package_args_serializes_to_galaxy_yml_keys() {
    let mut deps = IndexMap::new();
    deps.insert("community.general".into(), ">=8.0.0".into());
    deps.insert("ansible.posix".into(), "*".into());
    let args = PackageArgs {
        namespace: Some("plemeio".into()),
        name: Some("substrate".into()),
        version: Some("1.0.0".into()),
        description: Some("Substrate-side typed ansible collection".into()),
        authors: vec!["drzzln <a@b>".into()],
        license: vec!["MIT".into()],
        tags: vec!["substrate".into(), "fleet".into()],
        dependencies: deps,
        repository: Some("https://github.com/pleme-io/substrate".into()),
        build_ignore: vec!["tests/output".into(), "*.swp".into()],
        ..Default::default()
    };
    let v: serde_json::Value = serde_json::to_value(&args).unwrap();
    assert_eq!(v.get("namespace"), Some(&serde_json::json!("plemeio")));
    assert_eq!(v.get("name"), Some(&serde_json::json!("substrate")));
    assert_eq!(v.get("version"), Some(&serde_json::json!("1.0.0")));
    assert_eq!(v.get("license"), Some(&serde_json::json!(["MIT"])));
    let dv = v.get("dependencies").unwrap().as_object().unwrap();
    assert_eq!(dv.get("community.general"), Some(&serde_json::json!(">=8.0.0")));
    assert_eq!(v.get("build_ignore"), Some(&serde_json::json!(["tests/output", "*.swp"])));
}

#[test]
fn quirk_variants_round_trip() {
    for q in [
        AnsibleQuirk::DropDependency { collection: "broken.collection".into() },
        AnsibleQuirk::PinDependency {
            collection: "community.general".into(),
            version: "==8.0.0".into(),
        },
        AnsibleQuirk::BuildIgnore { path: "tmp".into() },
        AnsibleQuirk::SubstituteSource {
            file: "roles/x/tasks/main.yml".into(),
            from: "old".into(),
            to: "new".into(),
        },
    ] {
        let v: serde_json::Value = serde_json::to_value(&q).unwrap();
        assert!(v.get("kind").is_some());
        let back: AnsibleQuirk = serde_json::from_value(v).unwrap();
        assert_eq!(q, back);
    }
}

#[test]
fn build_spec_implements_spec_trait() {
    let mut packages = IndexMap::new();
    packages.insert(
        "plemeio.substrate-1.0.0".into(),
        PackageSpec {
            name: "plemeio.substrate".into(),
            version: "1.0.0".into(),
            args: PackageArgs {
                namespace: Some("plemeio".into()),
                name: Some("substrate".into()),
                version: Some("1.0.0".into()),
                ..Default::default()
            },
            quirks: vec![],
        },
    );
    let spec = BuildSpec {
        version: SCHEMA_VERSION,
        packages,
        root_package: "plemeio.substrate-1.0.0".into(),
        workspace_members: vec!["plemeio.substrate-1.0.0".into()],
    };
    assert_eq!(spec.schema_version(), SCHEMA_VERSION);
    assert_eq!(spec.root_key(), "plemeio.substrate-1.0.0");
    let violations = <AnsibleInvariants as Invariants>::check(&spec);
    assert!(violations.is_empty(), "minimal spec violated: {violations:?}");
}

#[test]
fn registry_is_empty_default() {
    assert!(<AnsibleQuirks as QuirkRegistry>::registered_names().is_empty());
}

#[test]
fn adapter_quirks_registry_envelope() {
    let a = gen_ansible::adapter::AnsibleAdapter;
    assert!(a.quirks_registry().is_empty());
}
