//! Typed build-spec surface tests. The coarse (v1 buildGoModule) shape
//! and the v2 incremental shape both implement `gen_types::Spec`; the
//! quirk enum round-trips; the empty registry is callable.

use gen_gomod::build_spec::coarse::{self, PackageArgs};
use gen_gomod::build_spec::{
    self, BuildTree, DepMode, EmbedSpec, GoPackageArgs, ModuleSpec, PackageKind, PackageSource,
    PackageSpec, Renderer, SCHEMA_VERSION,
};
use gen_gomod::invariants::GomodInvariants;
use gen_gomod::quirks::{GomodQuirk, GomodQuirks};
use gen_types::{Adapter, Invariants, QuirkRegistry, Spec};
use indexmap::IndexMap;
use std::collections::BTreeMap;

#[test]
fn coarse_package_args_serializes_to_build_go_module_keys() {
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
        GomodQuirk::SubstituteSource { file: "go.mod".into(), from: "x".into(), to: "y".into() },
    ] {
        let v: serde_json::Value = serde_json::to_value(&q).unwrap();
        assert!(v.get("kind").is_some());
        let back: GomodQuirk = serde_json::from_value(v).unwrap();
        assert_eq!(q, back);
    }
}

fn module_spec() -> ModuleSpec {
    ModuleSpec {
        module_path: "example.com/x".into(),
        go_version: "1.25".into(),
        toolchain: None,
        has_external_deps: false,
        dep_mode: DepMode::Vendored,
        vendor_hash: None,
    }
}

#[test]
fn v2_build_spec_implements_spec_trait() {
    let mut packages = BTreeMap::new();
    let key = "example.com/x/cmd/hello#linux-amd64".to_string();
    packages.insert(
        key.clone(),
        PackageSpec {
            import_path: "example.com/x/cmd/hello".into(),
            kind: PackageKind::Main,
            source: PackageSource::Vendored { relative_path: "cmd/hello".into() },
            tree: BuildTree::Target,
            go_files: vec!["main.go".into()],
            build_tags: vec![],
            embed: EmbedSpec::default(),
            imports: vec![],
            import_map: BTreeMap::new(),
            module: None,
            source_hash: "abc123".into(),
            args: GoPackageArgs::default(),
            quirks: vec![],
        },
    );
    let spec = build_spec::BuildSpec {
        version: SCHEMA_VERSION,
        renderer: Renderer::Incremental,
        module: module_spec(),
        packages,
        root_package: key.clone(),
        workspace_members: vec![key.clone()],
        target_resolves: None,
        go_sum_sha256: None,
    };
    assert_eq!(spec.schema_version(), SCHEMA_VERSION);
    assert_eq!(spec.root_key(), key);
    assert!(spec.renderer.is_incremental());
    let violations = <GomodInvariants as Invariants>::check(&spec);
    assert!(violations.is_empty(), "minimal v2 spec violated: {violations:?}");
}

#[test]
fn coarse_build_spec_still_implements_spec_trait() {
    let mut packages = IndexMap::new();
    packages.insert(
        "kubectl-1.29.0".into(),
        coarse::PackageSpec {
            name: "kubectl".into(),
            version: "1.29.0".into(),
            args: PackageArgs { pname: Some("kubectl".into()), ..Default::default() },
            quirks: vec![],
        },
    );
    let spec = coarse::BuildSpec {
        version: coarse::SCHEMA_VERSION,
        packages,
        root_package: "kubectl-1.29.0".into(),
        workspace_members: vec!["kubectl-1.29.0".into()],
    };
    assert_eq!(spec.schema_version(), coarse::SCHEMA_VERSION);
    assert_eq!(spec.root_key(), "kubectl-1.29.0");
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
