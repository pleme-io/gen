use gen_swift::build_spec::{BuildSpec, PackageArgs, PackageSpec, SCHEMA_VERSION};
use gen_swift::invariants::SwiftInvariants;
use gen_swift::quirks::{SwiftQuirk, SwiftQuirks};
use gen_types::{Adapter, Invariants, QuirkRegistry, Spec};
use indexmap::IndexMap;

#[test]
fn package_args_serializes_to_swift_package_keys() {
    let args = PackageArgs {
        pname: Some("AsyncHTTPClient".into()),
        version: Some("1.18.0".into()),
        swift_deps: Some("sha256-asyncdeps".into()),
        configuration: Some("release".into()),
        products: vec!["AsyncHTTPClient".into()],
        targets: vec!["AsyncHTTPClient".into()],
        swift_platform_version: Some("macOS 13".into()),
        ldflags: vec!["-l-some-lib".into()],
        pkg_config_deps: vec!["openssl".into()],
        do_check: Some(false),
        ..Default::default()
    };
    let v: serde_json::Value = serde_json::to_value(&args).unwrap();
    assert_eq!(v.get("pname"), Some(&serde_json::json!("AsyncHTTPClient")));
    assert_eq!(v.get("version"), Some(&serde_json::json!("1.18.0")));
    assert_eq!(
        v.get("swiftDeps"),
        Some(&serde_json::json!("sha256-asyncdeps"))
    );
    assert_eq!(v.get("configuration"), Some(&serde_json::json!("release")));
    assert_eq!(
        v.get("products"),
        Some(&serde_json::json!(["AsyncHTTPClient"]))
    );
    assert_eq!(
        v.get("targets"),
        Some(&serde_json::json!(["AsyncHTTPClient"]))
    );
    assert_eq!(
        v.get("swiftPlatformVersion"),
        Some(&serde_json::json!("macOS 13"))
    );
    assert_eq!(
        v.get("pkgConfigDeps"),
        Some(&serde_json::json!(["openssl"]))
    );
}

#[test]
fn quirk_variants_round_trip() {
    for q in [
        SwiftQuirk::PinToolchain {
            version: "5.10".into(),
        },
        SwiftQuirk::ForceConfiguration {
            configuration: "debug".into(),
        },
        SwiftQuirk::Ldflag {
            flag: "-lpthread".into(),
        },
        SwiftQuirk::SubstituteSource {
            file: "Package.swift".into(),
            from: "x".into(),
            to: "y".into(),
        },
    ] {
        let v: serde_json::Value = serde_json::to_value(&q).unwrap();
        assert!(v.get("kind").is_some());
        let back: SwiftQuirk = serde_json::from_value(v).unwrap();
        assert_eq!(q, back);
    }
}

#[test]
fn build_spec_implements_spec_trait() {
    let mut packages = IndexMap::new();
    packages.insert(
        "AsyncHTTPClient-1.18.0".into(),
        PackageSpec {
            name: "AsyncHTTPClient".into(),
            version: "1.18.0".into(),
            args: PackageArgs {
                pname: Some("AsyncHTTPClient".into()),
                version: Some("1.18.0".into()),
                ..Default::default()
            },
            quirks: vec![],
        },
    );
    let spec = BuildSpec {
        version: SCHEMA_VERSION,
        packages,
        root_package: "AsyncHTTPClient-1.18.0".into(),
        workspace_members: vec!["AsyncHTTPClient-1.18.0".into()],
    };
    assert_eq!(spec.schema_version(), SCHEMA_VERSION);
    assert_eq!(spec.root_key(), "AsyncHTTPClient-1.18.0");
    let violations = <SwiftInvariants as Invariants>::check(&spec);
    assert!(
        violations.is_empty(),
        "minimal spec violated: {violations:?}"
    );
}

#[test]
fn registry_is_empty_default() {
    assert!(<SwiftQuirks as QuirkRegistry>::registered_names().is_empty());
}

#[test]
fn adapter_quirks_registry_envelope() {
    let a = gen_swift::adapter::SwiftAdapter;
    assert!(a.quirks_registry().is_empty());
}
