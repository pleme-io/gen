use gen_bundler::build_spec::{BuildSpec, PackageArgs, PackageSpec, SCHEMA_VERSION};
use gen_bundler::invariants::{BundlerInvariants, Violation};
use gen_bundler::quirks::{BundlerQuirk, BundlerQuirks};
use gen_types::{Adapter, Invariants, QuirkRegistry, Spec};
use indexmap::IndexMap;

#[test]
fn build_spec_implements_spec_trait() {
    let mut packages = IndexMap::new();
    packages.insert(
        "demo-1.0.0".into(),
        PackageSpec {
            name: "demo".into(),
            version: "1.0.0".into(),
            args: PackageArgs {
                pname: Some("demo".into()),
                version: Some("1.0.0".into()),
                groups: vec!["default".into(), "production".into()],
                ..Default::default()
            },
            quirks: vec![],
        },
    );
    let spec = BuildSpec {
        version: SCHEMA_VERSION,
        packages,
        root_package: "demo-1.0.0".into(),
        workspace_members: vec!["demo-1.0.0".into()],
    };
    assert_eq!(spec.schema_version(), SCHEMA_VERSION);
    assert_eq!(spec.root_key(), "demo-1.0.0");
    assert_eq!(spec.member_keys(), vec!["demo-1.0.0"]);
    let args = spec.args_for("demo-1.0.0").unwrap();
    assert_eq!(args.groups, vec!["default", "production"]);
}

#[test]
fn package_args_serializes_with_canonical_bundler_keys() {
    let args = PackageArgs {
        pname: Some("foo".into()),
        version: Some("1.0.0".into()),
        gemfile: Some("./Gemfile".into()),
        lockfile: Some("./Gemfile.lock".into()),
        gemset: Some("./gemset.nix".into()),
        ruby: Some("ruby_3_3".into()),
        groups: vec!["default".into()],
        exes: vec!["app".into()],
        do_check: Some(false),
        ..Default::default()
    };
    let v: serde_json::Value = serde_json::to_value(&args).unwrap();
    assert_eq!(v.get("pname"), Some(&serde_json::json!("foo")));
    assert_eq!(v.get("gemfile"), Some(&serde_json::json!("./Gemfile")));
    assert_eq!(
        v.get("lockfile"),
        Some(&serde_json::json!("./Gemfile.lock"))
    );
    assert_eq!(v.get("gemset"), Some(&serde_json::json!("./gemset.nix")));
    assert_eq!(v.get("ruby"), Some(&serde_json::json!("ruby_3_3")));
    assert_eq!(v.get("groups"), Some(&serde_json::json!(["default"])));
    assert_eq!(v.get("exes"), Some(&serde_json::json!(["app"])));
    assert_eq!(
        v.get("doCheck"),
        Some(&serde_json::json!(false)),
        "doCheck must be camelCase to match nixpkgs convention"
    );
}

#[test]
fn quirk_variants_round_trip() {
    for q in [
        BundlerQuirk::PinRuby {
            version: "3.3".into(),
        },
        BundlerQuirk::SkipNativeBuild,
        BundlerQuirk::ExtraCflags {
            flags: "-mssse3".into(),
        },
        BundlerQuirk::SubstituteSource {
            file: "lib/x.rb".into(),
            from: "a".into(),
            to: "b".into(),
        },
        BundlerQuirk::OverrideSource {
            url: "https://r".into(),
        },
    ] {
        let v: serde_json::Value = serde_json::to_value(&q).unwrap();
        assert!(v.get("kind").is_some());
        let back: BundlerQuirk = serde_json::from_value(v).unwrap();
        assert_eq!(q, back);
    }
}

#[test]
fn invariants_fire_on_missing_fields() {
    let mut packages = IndexMap::new();
    packages.insert(
        "broken-1.0.0".into(),
        PackageSpec {
            name: "broken".into(),
            version: "1.0.0".into(),
            args: PackageArgs::default(),
            quirks: vec![],
        },
    );
    let spec = BuildSpec {
        version: SCHEMA_VERSION,
        packages,
        root_package: "broken-1.0.0".into(),
        workspace_members: vec!["broken-1.0.0".into()],
    };
    let violations = <BundlerInvariants as Invariants>::check(&spec);
    assert!(
        violations
            .iter()
            .any(|v| matches!(v, Violation::MissingPname { .. }))
    );
    assert!(
        violations
            .iter()
            .any(|v| matches!(v, Violation::MissingVersion { .. }))
    );
}

#[test]
fn registry_is_empty_default() {
    assert!(<BundlerQuirks as QuirkRegistry>::registered_names().is_empty());
}

#[test]
fn adapter_quirks_registry_envelope() {
    let a = gen_bundler::BundlerAdapter;
    assert!(a.quirks_registry().is_empty());
}
