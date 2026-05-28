use gen_poetry::build_spec::{BuildSpec, PackageArgs, PackageSpec, SCHEMA_VERSION};
use gen_poetry::invariants::PoetryInvariants;
use gen_poetry::quirks::{PoetryQuirk, PoetryQuirks};
use gen_types::{Adapter, Invariants, QuirkRegistry, Spec};
use indexmap::IndexMap;

#[test]
fn package_args_serializes_to_poetry2nix_keys() {
    let mut editable = IndexMap::new();
    editable.insert("my-lib".into(), "./libs/my-lib".into());
    let args = PackageArgs {
        project_dir: Some("./".into()),
        python: Some("python311".into()),
        groups: vec!["main".into(), "production".into()],
        extras: vec!["postgres".into()],
        prefer_wheels: Some(true),
        editable_package_sources: editable,
        do_check: Some(false),
        ..Default::default()
    };
    let v: serde_json::Value = serde_json::to_value(&args).unwrap();
    assert_eq!(v.get("projectDir"), Some(&serde_json::json!("./")));
    assert_eq!(v.get("python"), Some(&serde_json::json!("python311")));
    assert_eq!(v.get("groups"), Some(&serde_json::json!(["main", "production"])));
    assert_eq!(v.get("extras"), Some(&serde_json::json!(["postgres"])));
    assert_eq!(v.get("preferWheels"), Some(&serde_json::json!(true)));
    assert_eq!(v.get("doCheck"), Some(&serde_json::json!(false)));
    let eps = v.get("editablePackageSources").unwrap().as_object().unwrap();
    assert_eq!(eps.get("my-lib"), Some(&serde_json::json!("./libs/my-lib")));
}

#[test]
fn quirk_variants_round_trip() {
    for q in [
        PoetryQuirk::OverrideBuildSystem {
            package: "broken-pkg".into(),
            backend: "setuptools".into(),
        },
        PoetryQuirk::OverrideAttrs {
            package: "broken-pkg".into(),
            attr: "preBuild".into(),
            value: "true".into(),
        },
        PoetryQuirk::SkipCheck { package: "flaky-pkg".into() },
        PoetryQuirk::PreferWheel { package: "scipy".into(), prefer: true },
    ] {
        let v: serde_json::Value = serde_json::to_value(&q).unwrap();
        assert!(v.get("kind").is_some());
        let back: PoetryQuirk = serde_json::from_value(v).unwrap();
        assert_eq!(q, back);
    }
}

#[test]
fn build_spec_implements_spec_trait() {
    let mut packages = IndexMap::new();
    packages.insert(
        "myapp-0.1.0".into(),
        PackageSpec {
            name: "myapp".into(),
            version: "0.1.0".into(),
            args: PackageArgs {
                project_dir: Some("./".into()),
                ..Default::default()
            },
            quirks: vec![],
        },
    );
    let spec = BuildSpec {
        version: SCHEMA_VERSION,
        packages,
        root_package: "myapp-0.1.0".into(),
        workspace_members: vec!["myapp-0.1.0".into()],
    };
    assert_eq!(spec.schema_version(), SCHEMA_VERSION);
    assert_eq!(spec.root_key(), "myapp-0.1.0");
    let violations = <PoetryInvariants as Invariants>::check(&spec);
    assert!(violations.is_empty(), "minimal spec violated: {violations:?}");
}

#[test]
fn registry_is_empty_default() {
    assert!(<PoetryQuirks as QuirkRegistry>::registered_names().is_empty());
}

#[test]
fn adapter_quirks_registry_envelope() {
    let a = gen_poetry::adapter::PoetryAdapter;
    assert!(a.quirks_registry().is_empty());
}
