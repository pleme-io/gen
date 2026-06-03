use gen_gomod::build_spec::{BuildSpec, PackageArgs, PackageSpec, SCHEMA_VERSION};
use gen_gomod::invariants::GomodInvariants;
use gen_gomod::quirks::{GomodQuirk, GomodQuirks};
use gen_types::{Adapter, Invariants, QuirkRegistry, Spec};
use indexmap::IndexMap;

#[test]
fn package_args_serializes_to_build_go_module_keys() {
    let mut env = IndexMap::new();
    env.insert("CGO_ENABLED".into(), "0".into());
    env.insert("GOOS".into(), "linux".into());
    let args = PackageArgs {
        pname: Some("kubectl".into()),
        version: Some("1.29.0".into()),
        vendor_hash: Some("sha256-abc123".into()),
        proxy_vendor: Some(true),
        tags: vec!["netgo".into()],
        ldflags: vec!["-s".into(), "-w".into(), "-X main.version=1.29.0".into()],
        sub_packages: vec!["cmd/kubectl".into()],
        do_check: Some(false),
        env,
        ..Default::default()
    };
    let v: serde_json::Value = serde_json::to_value(&args).unwrap();
    assert_eq!(v.get("pname"), Some(&serde_json::json!("kubectl")));
    assert_eq!(v.get("vendorHash"), Some(&serde_json::json!("sha256-abc123")));
    assert_eq!(v.get("proxyVendor"), Some(&serde_json::json!(true)));
    assert_eq!(v.get("tags"), Some(&serde_json::json!(["netgo"])));
    assert_eq!(v.get("subPackages"), Some(&serde_json::json!(["cmd/kubectl"])));
    assert_eq!(v.get("doCheck"), Some(&serde_json::json!(false)));
    let envv = v.get("env").unwrap().as_object().unwrap();
    assert_eq!(envv.get("CGO_ENABLED"), Some(&serde_json::json!("0")));
}

#[test]
fn quirk_variants_round_trip() {
    for q in [
        GomodQuirk::ForceVendorHash { hash: "sha256-xyz".into() },
        GomodQuirk::BuildTag { tag: "fips".into() },
        GomodQuirk::Ldflag { flag: "-w".into() },
        GomodQuirk::CgoOff,
        GomodQuirk::SubstituteSource {
            file: "go.mod".into(),
            from: "x".into(),
            to: "y".into(),
        },
    ] {
        let v: serde_json::Value = serde_json::to_value(&q).unwrap();
        assert!(v.get("kind").is_some());
        let back: GomodQuirk = serde_json::from_value(v).unwrap();
        assert_eq!(q, back);
    }
}

#[test]
fn build_spec_implements_spec_trait() {
    let mut packages = IndexMap::new();
    packages.insert(
        "kubectl-1.29.0".into(),
        PackageSpec {
            name: "kubectl".into(),
            version: "1.29.0".into(),
            args: PackageArgs {
                pname: Some("kubectl".into()),
                version: Some("1.29.0".into()),
                ..Default::default()
            },
            has_external_deps: false,
            quirks: vec![],
        },
    );
    let spec = BuildSpec {
        version: SCHEMA_VERSION,
        packages,
        root_package: "kubectl-1.29.0".into(),
        // workspace_members are module paths (a package's `name`), keeping
        // member-list and package-set in agreement per the gomod invariants.
        workspace_members: vec!["kubectl".into()],
    };
    assert_eq!(spec.schema_version(), SCHEMA_VERSION);
    assert_eq!(spec.root_key(), "kubectl-1.29.0");
    let violations = <GomodInvariants as Invariants>::check(&spec);
    assert!(violations.is_empty(), "minimal spec violated: {violations:?}");
}

#[test]
fn registry_is_empty_default() {
    assert!(<GomodQuirks as QuirkRegistry>::registered_names().is_empty());
}

#[test]
fn adapter_quirks_registry_envelope() {
    let a = gen_gomod::adapter::GomodAdapter;
    assert!(a.quirks_registry().is_empty());
}
