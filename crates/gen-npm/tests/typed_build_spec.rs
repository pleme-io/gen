//! Locks the npm typed BuildSpec + PackageArgs shape against
//! buildNpmPackage's kwarg surface. Catches drift where someone
//! renames a field without updating the nixpkgs-side rename.

use gen_npm::build_spec::{BuildSpec, PackageArgs, PackageSpec, SCHEMA_VERSION};
use gen_npm::invariants::{NpmInvariants, Violation};
use gen_npm::quirks::{NpmQuirk, NpmQuirks};
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
    assert!(spec.args_for("demo-1.0.0").is_some());
    assert!(spec.args_for("not-there").is_none());
    assert!(spec.quirks_for("demo-1.0.0").is_empty());
}

#[test]
fn package_args_serializes_to_camel_case_buildnpmpackage_keys() {
    let args = PackageArgs {
        pname: Some("foo".into()),
        version: Some("1.0.0".into()),
        npm_deps_hash: Some("sha256-abc".into()),
        npm_build_script: Some("build".into()),
        dont_npm_build: Some(false),
        npm_flags: vec!["--legacy-peer-deps".into()],
        ..Default::default()
    };
    let v: serde_json::Value = serde_json::to_value(&args).unwrap();
    assert_eq!(v.get("pname"), Some(&serde_json::json!("foo")));
    assert_eq!(v.get("version"), Some(&serde_json::json!("1.0.0")));
    // Critical: these MUST be camelCase to match nixpkgs' buildNpmPackage.
    assert_eq!(
        v.get("npmDepsHash"),
        Some(&serde_json::json!("sha256-abc")),
        "spec must serialize npmDepsHash in camelCase"
    );
    assert_eq!(
        v.get("npmBuildScript"),
        Some(&serde_json::json!("build")),
        "spec must serialize npmBuildScript in camelCase"
    );
    assert_eq!(
        v.get("dontNpmBuild"),
        Some(&serde_json::json!(false)),
        "spec must serialize dontNpmBuild in camelCase"
    );
    assert_eq!(
        v.get("npmFlags"),
        Some(&serde_json::json!(["--legacy-peer-deps"])),
        "spec must serialize npmFlags in camelCase"
    );
}

#[test]
fn quirk_variants_serialize_with_kind_tag() {
    for q in [
        NpmQuirk::NpmInstallFlag { flag: "--legacy-peer-deps".into() },
        NpmQuirk::SkipPostinstall,
        NpmQuirk::PinNodejs { version: "20".into() },
        NpmQuirk::OverrideRegistry { url: "https://r".into() },
        NpmQuirk::SubstituteSource {
            file: "src/foo.js".into(),
            from: "a".into(),
            to: "b".into(),
        },
    ] {
        let v: serde_json::Value = serde_json::to_value(&q).unwrap();
        assert!(v.get("kind").is_some(), "quirk must carry `kind` tag: {q:?}");
        // Round-trip through serde.
        let back: NpmQuirk = serde_json::from_value(v).unwrap();
        assert_eq!(q, back);
    }
}

#[test]
fn invariants_catch_missing_pname() {
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
    let violations = <NpmInvariants as Invariants>::check(&spec);
    assert!(violations.iter().any(|v| matches!(v, Violation::MissingPname { .. })));
    assert!(violations.iter().any(|v| matches!(v, Violation::MissingVersion { .. })));
}

#[test]
fn invariants_catch_stale_schema_version() {
    let spec = BuildSpec {
        version: 0,
        packages: IndexMap::new(),
        root_package: "".into(),
        workspace_members: vec![],
    };
    let v = <NpmInvariants as Invariants>::check(&spec);
    assert!(v.iter().any(|x| matches!(x, Violation::StaleSchemaVersion { .. })));
}

#[test]
fn empty_quirk_registry_iterates_clean() {
    let names = <NpmQuirks as QuirkRegistry>::registered_names();
    assert!(names.is_empty(), "default registry should be empty");
}

#[test]
fn adapter_exposes_quirks_via_envelope() {
    let a = gen_npm::NpmAdapter;
    let entries = a.quirks_registry();
    assert!(entries.is_empty());
}
