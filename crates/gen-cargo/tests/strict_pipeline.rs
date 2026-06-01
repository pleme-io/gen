//! Strict build-spec DATA PIPELINE forcing-functions (schema v10).
//!
//! The decided model (enforced mechanically by the tests below):
//!
//!   1. gen (Rust) computes everything; `Cargo.build-spec.json` carries
//!      ONLY what the Nix consumer reads — no dead/redundant restatement.
//!   2. The spec MUST round-trip losslessly:
//!      `from_slice::<BuildSpec>(to_vec(&spec))` succeeds for ANY valid
//!      spec, and re-serializing yields byte-identical output.
//!   3. Schema is v10 — `target_resolves` is the COMPACT shape:
//!      `base` (cross-target-identical per-crate edges, stored once) +
//!      `targets[triple].overrides` (only the crates that differ on that
//!      triple). Substrate decodes `base // overrides[triple]`.
//!
//! These tests are the strictness layer. They catch the whole
//! skip-without-default bug class FOREVER (`roundtrip_lossless`), lock
//! the v9 union-drop win against regression (`no_dead_dependencies_union`,
//! now asserting on the compact `base` + `overrides` paths), prove the
//! dropped Normal∪Build union carried zero unique data (`lossless_split`),
//! prove the v10 compaction is itself lossless (`compact_is_lossless`),
//! prove `base` holds only universal-identical edges
//! (`base_holds_only_universal_identical`), and pin the schema version on
//! both the const and a freshly-generated spec (`schema_is_v10`).
//!
//! Two spec sources are used:
//!   - A freshly-generated fixture spec (tiny single-crate workspace) —
//!     carries the CURRENT `SCHEMA_VERSION` and the v9 union-drop
//!     behavior. Network-gated like `spec_invariants_e2e.rs`.
//!   - The repo's committed `Cargo.build-spec.json` (the gen workspace's
//!     own multi-crate spec) — a real, representative spec proving the
//!     actual on-disk artifact deserializes (Task C equivalent proof,
//!     no network required).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use gen_cargo::build_spec::{self, BuildSpec, CrateSource};
use tempfile::TempDir;

/// Build a tiny single-crate workspace + lockfile, identical in shape to
/// `spec_invariants_e2e.rs::write_fixture`. Returns None-equivalent (an
/// empty-lock TempDir) when cargo can't produce a lockfile; callers skip.
fn write_fixture() -> TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "fixture-crate"
version = "0.1.0"
edition = "2024"

[dependencies]
serde = { version = "1", features = ["derive"] }
"#,
    )
    .unwrap();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("src/lib.rs"),
        "pub fn hello() -> &'static str { \"hello\" }\n",
    )
    .unwrap();
    let _ = Command::new("cargo")
        .args(["generate-lockfile", "--offline"])
        .current_dir(root)
        .output();
    if !root.join("Cargo.lock").exists() {
        let _ = Command::new("cargo")
            .args(["generate-lockfile"])
            .current_dir(root)
            .output();
    }
    dir
}

/// Generate a fresh in-memory spec from a fixture, or `None` when the
/// environment can't produce a lockfile (hermetic sandbox). Callers that
/// need a fresh spec early-return when this is `None`.
fn generate_fixture_spec() -> Option<(TempDir, BuildSpec)> {
    let dir = write_fixture();
    if !dir.path().join("Cargo.lock").exists() {
        eprintln!("skipping: no Cargo.lock could be generated in fixture");
        return None;
    }
    let spec = build_spec::generate(dir.path()).expect("spec generation");
    Some((dir, spec))
}

/// Path to the gen workspace's committed `Cargo.build-spec.json`. The
/// test binary runs with CWD = the crate dir (gen-cargo), so walk up to
/// the workspace root.
fn committed_spec_path() -> Option<PathBuf> {
    // CARGO_MANIFEST_DIR = .../gen/crates/gen-cargo ; workspace root is
    // two levels up.
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest_dir.parent()?.parent()?;
    let p = root.join("Cargo.build-spec.json");
    p.is_file().then_some(p)
}

// ---------------------------------------------------------------------
// Task B.1 — roundtrip_lossless
// ---------------------------------------------------------------------
//
// Take a representative BuildSpec, serialize, deserialize, assert Ok;
// then assert the re-serialized bytes equal the first serialization
// (idempotent round-trip). This is THE forcing function that catches
// every skip-without-default field forever — a field with
// `skip_serializing_if` but no `#[serde(default)]` makes the
// `from_slice` step fail with `missing field <name>` the moment that
// field is empty.

fn assert_roundtrip_lossless(spec: &BuildSpec, label: &str) {
    let first = serde_json::to_vec(spec)
        .unwrap_or_else(|e| panic!("{label}: initial serialize failed: {e}"));
    let parsed: BuildSpec = serde_json::from_slice(&first).unwrap_or_else(|e| {
        panic!(
            "{label}: round-trip deserialize FAILED — a field has \
             skip_serializing[_if] without #[serde(default)]: {e}"
        )
    });
    let second = serde_json::to_vec(&parsed)
        .unwrap_or_else(|e| panic!("{label}: re-serialize failed: {e}"));
    assert_eq!(
        first, second,
        "{label}: round-trip is not idempotent — \
         serialize→deserialize→serialize changed the bytes"
    );
}

#[test]
fn roundtrip_lossless_on_generated_fixture() {
    let Some((_dir, spec)) = generate_fixture_spec() else {
        return;
    };
    assert_roundtrip_lossless(&spec, "generated-fixture");
}

#[test]
fn roundtrip_lossless_on_committed_spec() {
    // Equivalent of Task C: the ACTUAL committed Cargo.build-spec.json
    // deserializes into a full BuildSpec without a missing-field error,
    // and re-serializes idempotently. This is the regression guard for
    // the `gen lock --update` full-spec deserialize path
    // (lock_lifecycle::update -> serde_json::from_slice::<BuildSpec>).
    let Some(path) = committed_spec_path() else {
        eprintln!("skipping: no committed Cargo.build-spec.json at workspace root");
        return;
    };
    let bytes = fs::read(&path).expect("read committed spec");
    let spec: BuildSpec = serde_json::from_slice(&bytes).unwrap_or_else(|e| {
        panic!(
            "committed {} failed full-BuildSpec deserialize (the \
             `gen lock --update` bug class): {e}",
            path.display()
        )
    });
    assert_roundtrip_lossless(&spec, "committed-spec");
}

// ---------------------------------------------------------------------
// Task B.2 — no_dead_dependencies_union (v9 size win)
// ---------------------------------------------------------------------
//
// Serialize a BuildSpec to JSON and assert there is NO `dependencies`
// key on any top-level crates[*] NOR on any edge-set inside the compact
// `target_resolves` (neither `target_resolves.base[*]` nor
// `target_resolves.targets[*].overrides[*]`), while runtime_dependencies
// AND build_dependencies DO appear. Locks the v9 union-drop against
// regression under the v10 compact shape (the per-triple `crates` path
// no longer exists). Operates on a FRESHLY-GENERATED spec so it reflects
// the current `#[serde(skip_serializing)]` behavior.

#[test]
fn no_dead_dependencies_union() {
    let Some((_dir, spec)) = generate_fixture_spec() else {
        return;
    };
    let value: serde_json::Value =
        serde_json::to_value(&spec).expect("serialize spec to Value");

    let crates = value
        .get("crates")
        .and_then(|c| c.as_object())
        .expect("crates object");
    assert!(!crates.is_empty(), "fixture produced no crates");

    for (key, c) in crates {
        let obj = c.as_object().expect("crate is an object");
        assert!(
            !obj.contains_key("dependencies"),
            "crates[{key}] still serializes the redundant `dependencies` \
             union — v9 drops it (the single largest size win)"
        );
        assert!(
            obj.contains_key("runtime_dependencies"),
            "crates[{key}] missing `runtime_dependencies` — the Nix \
             consumer reads this directly"
        );
        assert!(
            obj.contains_key("build_dependencies"),
            "crates[{key}] missing `build_dependencies` — the Nix \
             consumer reads this directly"
        );
    }

    // Same assertion inside the compact target_resolves (v10): walk
    // `base[*]` AND `targets[*].overrides[*]` — the per-triple `crates`
    // path no longer exists.
    if let Some(tr) = value.get("target_resolves").and_then(|r| r.as_object()) {
        // Assert every edge-set object obeys the v9 union-drop.
        let assert_edges = |label: &str, edges: &serde_json::Map<String, serde_json::Value>| {
            assert!(
                !edges.contains_key("dependencies"),
                "{label} still serializes the redundant `dependencies` \
                 union — v9 drops it"
            );
            assert!(
                edges.contains_key("runtime_dependencies"),
                "{label} missing `runtime_dependencies`"
            );
            assert!(
                edges.contains_key("build_dependencies"),
                "{label} missing `build_dependencies`"
            );
        };

        // base[*]
        if let Some(base) = tr.get("base").and_then(|b| b.as_object()) {
            for (key, c) in base {
                let obj = c.as_object().expect("base crate is an object");
                assert_edges(&format!("target_resolves.base[{key}]"), obj);
            }
        }

        // targets[triple].overrides[*]
        if let Some(targets) = tr.get("targets").and_then(|t| t.as_object()) {
            for (triple, to) in targets {
                let overrides = to
                    .get("overrides")
                    .and_then(|o| o.as_object())
                    .unwrap_or_else(|| {
                        panic!("target_resolves.targets[{triple}].overrides not an object")
                    });
                for (key, c) in overrides {
                    let obj = c.as_object().expect("override crate is an object");
                    assert_edges(
                        &format!("target_resolves.targets[{triple}].overrides[{key}]"),
                        obj,
                    );
                }
            }
        }
    }
}

// ---------------------------------------------------------------------
// Task B.3 — lossless_split (the union carried zero unique data)
// ---------------------------------------------------------------------
//
// For every crate, the in-memory `dependencies` union (Normal∪Build)
// equals the set-union of `runtime_dependencies` + `build_dependencies`
// by package_key. Proves dropping the union from JSON loses nothing.
// Operates on the in-memory struct BEFORE serialization (the union is
// `#[serde(skip_serializing)]`, so it's absent from JSON but present
// in the generated value).

#[test]
fn lossless_split_union_equals_runtime_plus_build() {
    let Some((_dir, spec)) = generate_fixture_spec() else {
        return;
    };

    use std::collections::BTreeSet;
    for (key, c) in &spec.crates {
        let union_keys: BTreeSet<&str> =
            c.dependencies.iter().map(|d| d.package_key.as_str()).collect();
        let split_keys: BTreeSet<&str> = c
            .runtime_dependencies
            .iter()
            .chain(c.build_dependencies.iter())
            .map(|d| d.package_key.as_str())
            .collect();
        assert_eq!(
            union_keys, split_keys,
            "crates[{key}]: the dropped `dependencies` union by package_key \
             differs from runtime_dependencies ∪ build_dependencies — the \
             v9 drop would lose data"
        );
    }
}

// ---------------------------------------------------------------------
// Task B.4 — schema_is_v10
// ---------------------------------------------------------------------

#[test]
fn schema_is_v10_const() {
    assert_eq!(
        build_spec::SCHEMA_VERSION,
        10,
        "SCHEMA_VERSION must be 10 for the decided model"
    );
}

#[test]
fn schema_is_v10_on_freshly_generated_spec() {
    let Some((_dir, spec)) = generate_fixture_spec() else {
        return;
    };
    assert_eq!(
        spec.version, 10,
        "a freshly generated spec's `version` must equal 10"
    );
    assert_eq!(
        spec.version,
        build_spec::SCHEMA_VERSION,
        "generated spec version must equal SCHEMA_VERSION"
    );
}

// ---------------------------------------------------------------------
// Belt-and-suspenders: a hand-built minimal spec with EVERY skipped
// field at its empty value still round-trips. This runs with no network
// and exercises the exact `crateRenames` (+ siblings) absence that broke
// `gen lock --update` on main.
// ---------------------------------------------------------------------

#[test]
fn minimal_empty_spec_roundtrips_without_network() {
    use gen_cargo::build_spec::{CrateSpec, WorkspaceSpec};
    use indexmap::IndexMap;

    // A crate whose build_rust_crate_args are entirely default (so every
    // skip_serializing_if fires) and whose collections are empty. Before
    // the fix, serializing then deserializing this errored with
    // `missing field crateRenames`.
    let mut crates = IndexMap::new();
    crates.insert(
        "x-0.1.0".to_string(),
        CrateSpec {
            name: "x".to_string(),
            version: "0.1.0".to_string(),
            edition: "2021".to_string(),
            source: CrateSource::Path {
                relative_path: ".".to_string(),
            },
            features: Vec::new(),
            proc_macro: false,
            build_script: None,
            links: None,
            quirks: Vec::new(),
            binaries: Vec::new(),
            lib_target: None,
            dependencies: Vec::new(),
            runtime_dependencies: Vec::new(),
            build_dependencies: Vec::new(),
            crate_renames: IndexMap::new(),
            build_rust_crate_args: Default::default(),
        },
    );

    let spec = BuildSpec {
        version: build_spec::SCHEMA_VERSION,
        workspace: WorkspaceSpec {
            root: "/tmp/x".to_string(),
            members: Vec::new(),
        },
        crates,
        root_crate: "x-0.1.0".to_string(),
        workspace_members: vec!["x-0.1.0".to_string()],
        flake_metadata: IndexMap::new(),
        target_resolves: None,
        cargo_lock_sha256: None,
    };

    assert_roundtrip_lossless(&spec, "minimal-empty");
}

// ---------------------------------------------------------------------
// v10 compaction: lossless split + base-correctness
// ---------------------------------------------------------------------
//
// These exercise the `CompactTargetResolves` split/expand round-trip on
// a hand-built multi-target fixture (no network). The fixture is shaped
// to exercise all three cases the split must handle:
//   - `common`   — identical edges on every target → lands in `base`.
//   - `differing`— present on every target but edges DIFFER on one
//                  target → must NOT be in `base`; full edges in each
//                  target's overrides.
//   - `linux-only`— present on only one target → not in `base`; in that
//                  one target's overrides.

use gen_cargo::build_spec::{
    CompactTargetResolves, CrateDepSpec, CrateTargetEdges, DepKind, TargetResolve,
};
use indexmap::IndexMap;

fn edge(pkg_key: &str, feature: &str) -> CrateTargetEdges {
    CrateTargetEdges {
        dependencies: Vec::new(),
        runtime_dependencies: vec![CrateDepSpec {
            name: pkg_key.split('-').next().unwrap_or(pkg_key).to_string(),
            package_key: pkg_key.to_string(),
            kind: DepKind::Normal,
            features: vec![feature.to_string()],
            uses_default_features: true,
            optional: false,
            target: None,
            tree: Default::default(),
        }],
        build_dependencies: Vec::new(),
        features: vec![feature.to_string()],
    }
}

/// A multi-target full map covering the three split cases.
fn fixture_full() -> IndexMap<String, TargetResolve> {
    let mk = |differing_feature: &str, include_linux_only: bool| {
        let mut crates: IndexMap<String, CrateTargetEdges> = IndexMap::new();
        // identical on every target
        crates.insert("common-1.0.0".to_string(), edge("dep-1.0.0", "std"));
        // present everywhere, but edges vary by target
        crates.insert(
            "differing-1.0.0".to_string(),
            edge("dep-1.0.0", differing_feature),
        );
        if include_linux_only {
            crates.insert("linux-only-1.0.0".to_string(), edge("dep-1.0.0", "epoll"));
        }
        TargetResolve { crates }
    };

    let mut full: IndexMap<String, TargetResolve> = IndexMap::new();
    full.insert("aarch64-apple-darwin".to_string(), mk("fsevent", false));
    full.insert("x86_64-apple-darwin".to_string(), mk("fsevent", false));
    full.insert(
        "x86_64-unknown-linux-gnu".to_string(),
        mk("inotify", true),
    );
    full
}

#[test]
fn compact_is_lossless() {
    let full = fixture_full();
    let compact = CompactTargetResolves::from_full(full.clone());
    let expanded = compact.expand();

    // Same set of triples.
    let full_triples: std::collections::BTreeSet<&String> = full.keys().collect();
    let exp_triples: std::collections::BTreeSet<&String> = expanded.keys().collect();
    assert_eq!(
        full_triples, exp_triples,
        "expand() must reconstruct exactly the original triples"
    );

    // Per triple: identical key set AND identical edges per key.
    for (triple, original) in &full {
        let reconstructed = expanded
            .get(triple)
            .unwrap_or_else(|| panic!("expand() dropped triple {triple}"));
        let orig_keys: std::collections::BTreeSet<&String> = original.crates.keys().collect();
        let rec_keys: std::collections::BTreeSet<&String> =
            reconstructed.crates.keys().collect();
        assert_eq!(
            orig_keys, rec_keys,
            "triple {triple}: base // overrides did not reconstruct the original key set"
        );
        for (key, orig_edges) in &original.crates {
            let rec_edges = reconstructed
                .crates
                .get(key)
                .unwrap_or_else(|| panic!("triple {triple}: missing crate {key} after expand"));
            assert_eq!(
                orig_edges, rec_edges,
                "triple {triple}: crate {key} edges differ after compact→expand round-trip"
            );
        }
    }
}

#[test]
fn base_holds_only_universal_identical() {
    let full = fixture_full();
    let compact = CompactTargetResolves::from_full(full.clone());

    // Only `common` is identical across every target → only it is in base.
    let base_keys: std::collections::BTreeSet<&String> = compact.base.keys().collect();
    let expected: std::collections::BTreeSet<String> = ["common-1.0.0".to_string()].into();
    let expected_refs: std::collections::BTreeSet<&String> = expected.iter().collect();
    assert_eq!(
        base_keys, expected_refs,
        "base must contain exactly the universally-present, byte-identical crates"
    );

    // Every base key appears in EVERY target's effective set and is
    // identical to the base copy.
    for (key, base_edges) in &compact.base {
        for (triple, resolve) in &full {
            let target_edges = resolve.crates.get(key).unwrap_or_else(|| {
                panic!("base key {key} absent from target {triple} — not universal")
            });
            assert_eq!(
                base_edges, target_edges,
                "base key {key} differs from its copy on target {triple}"
            );
        }
    }

    // No base key also appears in any overrides set.
    for (triple, to) in &compact.targets {
        for key in to.overrides.keys() {
            assert!(
                !compact.base.contains_key(key),
                "crate {key} is in BOTH base and target {triple} overrides — \
                 a crate must be in exactly one place"
            );
        }
    }

    // The differing + linux-only crates landed in overrides (not base):
    // differing on every target, linux-only on its single target.
    let darwin = compact
        .targets
        .get("aarch64-apple-darwin")
        .expect("darwin target present");
    assert!(
        darwin.overrides.contains_key("differing-1.0.0"),
        "the per-target-differing crate must be in overrides"
    );
    assert!(
        !darwin.overrides.contains_key("linux-only-1.0.0"),
        "linux-only crate must NOT appear in a darwin override"
    );
    let linux = compact
        .targets
        .get("x86_64-unknown-linux-gnu")
        .expect("linux target present");
    assert!(
        linux.overrides.contains_key("linux-only-1.0.0"),
        "linux-only crate must be in the linux override set"
    );
}

#[test]
fn single_target_compaction_is_all_base_empty_overrides() {
    // Single-target spec: base = all crates, every overrides empty,
    // still lossless. The reserved-keys invariant holds (no triple is
    // named `base`/`targets`).
    let mut full: IndexMap<String, TargetResolve> = IndexMap::new();
    let mut crates: IndexMap<String, CrateTargetEdges> = IndexMap::new();
    crates.insert("a-1.0.0".to_string(), edge("dep-1.0.0", "std"));
    crates.insert("b-2.0.0".to_string(), edge("dep-1.0.0", "alloc"));
    full.insert(
        "aarch64-apple-darwin".to_string(),
        TargetResolve { crates },
    );

    let compact = CompactTargetResolves::from_full(full.clone());
    assert_eq!(
        compact.base.len(),
        2,
        "single-target: every crate is universal-identical → all in base"
    );
    let to = compact
        .targets
        .get("aarch64-apple-darwin")
        .expect("the single target present");
    assert!(
        to.overrides.is_empty(),
        "single-target: overrides must be empty"
    );
    // Lossless.
    assert_eq!(compact.expand(), full);
}
