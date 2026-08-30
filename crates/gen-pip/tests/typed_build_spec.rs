use gen_pip::build_spec::{BuildSpec, PackageArgs, PackageSpec, SCHEMA_VERSION};
use gen_pip::invariants::PipInvariants;
use gen_pip::quirks::{PipQuirk, PipQuirks};
use gen_types::{Adapter, Invariants, QuirkRegistry, Spec};
use indexmap::IndexMap;

#[test]
fn package_args_serializes_to_build_python_package_keys() {
    let args = PackageArgs {
        pname: Some("rich".into()),
        version: Some("13.7.0".into()),
        pyproject: Some(true),
        build_system: vec!["poetry-core".into()],
        propagated_build_inputs: vec!["markdown-it-py".into(), "pygments".into()],
        native_build_inputs: vec!["python3Packages.poetry-core".into()],
        do_check: Some(false),
        python_imports_check: vec!["rich".into(), "rich.console".into()],
        ..Default::default()
    };
    let v: serde_json::Value = serde_json::to_value(&args).unwrap();
    assert_eq!(v.get("pname"), Some(&serde_json::json!("rich")));
    assert_eq!(v.get("version"), Some(&serde_json::json!("13.7.0")));
    assert_eq!(v.get("pyproject"), Some(&serde_json::json!(true)));
    assert_eq!(
        v.get("build-system"),
        Some(&serde_json::json!(["poetry-core"]))
    );
    assert_eq!(
        v.get("propagatedBuildInputs"),
        Some(&serde_json::json!(["markdown-it-py", "pygments"]))
    );
    assert_eq!(
        v.get("nativeBuildInputs"),
        Some(&serde_json::json!(["python3Packages.poetry-core"]))
    );
    assert_eq!(v.get("doCheck"), Some(&serde_json::json!(false)));
    assert_eq!(
        v.get("pythonImportsCheck"),
        Some(&serde_json::json!(["rich", "rich.console"]))
    );
}

#[test]
fn empty_args_serializes_to_empty_object() {
    let v: serde_json::Value = serde_json::to_value(PackageArgs::default()).unwrap();
    let obj = v.as_object().unwrap();
    assert!(
        obj.is_empty() || obj.values().all(|val| val.is_null()),
        "default PackageArgs should serialize to {{}} or all-null, got: {v}"
    );
}

#[test]
fn quirk_variants_round_trip() {
    for q in [
        PipQuirk::PinInterpreter {
            python: "python311".into(),
        },
        PipQuirk::SkipCheck,
        PipQuirk::DropRequires {
            package: "broken-dep".into(),
        },
        PipQuirk::SubstituteSource {
            file: "setup.py".into(),
            from: "old".into(),
            to: "new".into(),
        },
    ] {
        let v: serde_json::Value = serde_json::to_value(&q).unwrap();
        assert!(v.get("kind").is_some());
        let back: PipQuirk = serde_json::from_value(v).unwrap();
        assert_eq!(q, back);
    }
}

#[test]
fn build_spec_implements_spec_trait() {
    let mut packages = IndexMap::new();
    packages.insert(
        "rich-13.7.0".into(),
        PackageSpec {
            name: "rich".into(),
            version: "13.7.0".into(),
            args: PackageArgs {
                pname: Some("rich".into()),
                version: Some("13.7.0".into()),
                ..Default::default()
            },
            quirks: vec![],
        },
    );
    let spec = BuildSpec {
        version: SCHEMA_VERSION,
        packages,
        root_package: "rich-13.7.0".into(),
        workspace_members: vec!["rich-13.7.0".into()],
    };
    assert_eq!(spec.schema_version(), SCHEMA_VERSION);
    assert_eq!(spec.root_key(), "rich-13.7.0");
    let violations = <PipInvariants as Invariants>::check(&spec);
    assert!(
        violations.is_empty(),
        "minimal spec violated: {violations:?}"
    );
}

#[test]
fn registry_is_empty_default() {
    assert!(<PipQuirks as QuirkRegistry>::registered_names().is_empty());
}

#[test]
fn adapter_quirks_registry_envelope() {
    let a = gen_pip::adapter::PipAdapter;
    assert!(a.quirks_registry().is_empty());
}
