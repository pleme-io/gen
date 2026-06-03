//! Generation + adapter-verb gates for gen-gomod.
//!
//! STEP 1 gate: `generate` parses a go.mod+go.sum testdata fixture into
//! a populated `BuildSpec`.
//! STEP 3 gate: the `Adapter::build` trait surface succeeds on the
//! fixture and emits the typed envelope.
//! STEP 4 gate: `generate_and_write_with` (mock prefetcher) emits both
//! `Go.build-spec.json` + `Go.gen.lock`, and the delta round-trips.

use std::path::Path;

use gen_gomod::adapter::GomodAdapter;
use gen_gomod::build_spec;
use gen_gomod::gen_delta::{GoGenDelta, sha256_hex};
use gen_gomod::vendor_prefetcher::{MockVendorPrefetcher, PrefetchedHash};
use gen_types::{Adapter, AdapterCtx, GenDeltaArtifact};

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/simple-module");

// ── STEP 1 ──────────────────────────────────────────────────────────
#[test]
fn generate_populates_spec_from_fixture() {
    let spec = build_spec::generate(Path::new(FIXTURE)).expect("generate succeeds");
    assert_eq!(spec.version, build_spec::SCHEMA_VERSION);
    assert_eq!(spec.root_package, "widget");
    assert_eq!(spec.workspace_members, vec!["github.com/example/widget"]);

    let pkg = spec.packages.get("widget").expect("widget package present");
    assert_eq!(pkg.name, "github.com/example/widget");
    assert_eq!(pkg.args.pname.as_deref(), Some("widget"));
    assert_eq!(pkg.args.version.as_deref(), Some("0.0.0"));
    // The fixture has external requires → needs a vendorHash.
    assert!(pkg.has_external_deps);
    // proxyVendor is left at the nixpkgs default (false / unset) so the
    // vendorHash matches the default go-mod-vendor tree.
    assert_eq!(pkg.args.proxy_vendor, None);
    // vendorHash is uncomputed at generate-time (hermetic).
    assert!(pkg.args.vendor_hash.is_none());
}

#[test]
fn leaf_module_has_no_external_deps() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("go.mod"), "module example.com/leaf\ngo 1.21\n").unwrap();
    let spec = build_spec::generate(tmp.path()).unwrap();
    let pkg = spec.packages.get("leaf").unwrap();
    assert!(!pkg.has_external_deps);
    assert_eq!(pkg.args.proxy_vendor, None);
    assert!(pkg.args.vendor_hash.is_none());
}

// ── STEP 3 ──────────────────────────────────────────────────────────
#[test]
fn adapter_build_succeeds_on_fixture() {
    let a = GomodAdapter;
    let ctx = AdapterCtx {
        workspace_root: Path::new(FIXTURE).to_path_buf(),
        target: None,
    };
    let envelope = a.build(&ctx).expect("build succeeds");
    assert_eq!(envelope.ecosystem, "gomod");
    assert_eq!(envelope.schema_version, build_spec::SCHEMA_VERSION);
    // The data payload is the serialized BuildSpec.
    let root_pkg = envelope.data.get("root_package").and_then(|v| v.as_str());
    assert_eq!(root_pkg, Some("widget"));
}

#[test]
fn adapter_confirm_holds_on_hermetic_spec() {
    let a = GomodAdapter;
    let ctx = AdapterCtx {
        workspace_root: Path::new(FIXTURE).to_path_buf(),
        target: None,
    };
    let report = a.confirm(&ctx).expect("confirm succeeds");
    // vendor-hash-missing is expected on the hermetic build path and is
    // filtered out of broken invariants.
    assert!(
        report.invariants_broken.is_empty(),
        "unexpected broken invariants: {:?}",
        report.invariants_broken
    );
}

// ── STEP 4 ──────────────────────────────────────────────────────────
#[test]
fn generate_and_write_emits_both_artifacts() {
    // Copy the fixture into a temp dir so the write is hermetic.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    std::fs::copy(
        Path::new(FIXTURE).join("go.mod"),
        root.join("go.mod"),
    )
    .unwrap();
    std::fs::copy(
        Path::new(FIXTURE).join("go.sum"),
        root.join("go.sum"),
    )
    .unwrap();

    // Inject a mock prefetcher so no `go` subprocess runs.
    let mock = MockVendorPrefetcher::new();
    let fake = PrefetchedHash::from_digest([42u8; 32]);
    mock.insert(root, fake.clone());

    let out = build_spec::generate_and_write_with(root, &mock).expect("write succeeds");
    assert!(out.ends_with("Go.build-spec.json"));
    assert!(root.join("Go.build-spec.json").exists());
    assert!(root.join("Go.gen.lock").exists());

    // The build-spec carries the mock vendorHash.
    let spec_text = std::fs::read_to_string(root.join("Go.build-spec.json")).unwrap();
    let spec: serde_json::Value = serde_json::from_str(&spec_text).unwrap();
    let vh = spec["packages"]["widget"]["args"]["vendorHash"]
        .as_str()
        .unwrap();
    assert_eq!(vh, fake.sri);

    // The delta round-trips + ties to the go.sum hash.
    let delta_text = std::fs::read_to_string(root.join("Go.gen.lock")).unwrap();
    let delta: GoGenDelta = serde_json::from_str(&delta_text).unwrap();
    assert_eq!(GoGenDelta::FILENAME, "Go.gen.lock");
    let go_sum_bytes = std::fs::read(root.join("go.sum")).unwrap();
    assert_eq!(delta.go_sum_sha256, sha256_hex(&go_sum_bytes));
    assert_eq!(
        delta.per_package["widget"].vendor_hash.as_deref(),
        Some(fake.sri.as_str())
    );
}
