//! Determinism canonicalization — cross-platform byte-stability proof.
//!
//! `gen build` resolves per target via `cargo metadata --filter-platform`;
//! cargo's resolve-traversal + `meta.packages` iteration order is NOT a stable
//! cross-platform contract (linux CI ≠ darwin). Two build hosts therefore differ
//! ONLY in the insertion order of the spec's maps + resolve-ordered Vecs. So
//! "byte-identical output under arbitrary permutation of those orders" IS the
//! cross-platform byte-stability guarantee — provable on ONE host by adversarial
//! shuffling, no second OS needed.
//!
//! Maps are `BTreeMap` (canonical by construction); the Vecs are canonicalized
//! by `BuildSpec::canonicalize()`. These tests prove: (1) shuffle → canonicalize
//! → byte-identical for the full spec, (2) the slim delta inherits it, (3)
//! canonicalize is idempotent, (4) the sort is content-preserving (a permutation
//! of an unchanged multiset).

use gen_cargo::build_spec::{
    BuildSpec, CrateDepSpec, CrateSpec, CrateSource, CrateTargetEdges, DepKind,
    TargetResolve, WorkspaceSpec,
};
use gen_cargo::gen_delta::GenDelta;
use gen_cargo::gen_delta::GenDeltaArtifact;
use std::collections::BTreeMap;

fn dep(name: &str, version: &str, kind: DepKind, target: Option<&str>) -> CrateDepSpec {
    CrateDepSpec {
        name: name.to_string(),
        package_key: format!("{name}-{version}"),
        kind,
        features: vec!["z-feat".to_string(), "a-feat".to_string()],
        uses_default_features: true,
        optional: false,
        target: target.map(str::to_string),
        tree: Default::default(),
    }
}

fn edges() -> CrateTargetEdges {
    CrateTargetEdges {
        dependencies: Vec::new(),
        runtime_dependencies: vec![
            dep("zlib", "1.0.0", DepKind::Normal, None),
            dep("alpha", "2.0.0", DepKind::Normal, None),
            dep("libc", "0.2.0", DepKind::Normal, Some("cfg(unix)")),
            dep("mid", "1.0.0", DepKind::Build, None),
        ],
        build_dependencies: vec![
            dep("cc", "1.0.0", DepKind::Build, None),
            dep("bindgen", "0.1.0", DepKind::Build, None),
        ],
        features: vec!["std".to_string(), "alloc".to_string(), "default".to_string()],
    }
}

fn crate_spec(name: &str, version: &str) -> CrateSpec {
    CrateSpec {
        name: name.to_string(),
        version: version.to_string(),
        edition: "2021".to_string(),
        source: CrateSource::Registry {
            url: "https://crates.io/x".to_string(),
            sha256: "deadbeef".to_string(),
            name_with_ext: format!("{name}.tar.gz"),
        },
        features: vec!["std".to_string(), "alloc".to_string()],
        proc_macro: false,
        build_script: None,
        links: None,
        quirks: Vec::new(),
        binaries: Vec::new(),
        lib_target: None,
        dependencies: Vec::new(),
        runtime_dependencies: edges().runtime_dependencies,
        build_dependencies: edges().build_dependencies,
        crate_renames: BTreeMap::new(),
        build_rust_crate_args: Default::default(),
    }
}

/// A multi-crate, multi-target spec with deliberately UNSORTED insertion order
/// (z before a, etc.) so canonicalization has real work to do.
fn fixture() -> BuildSpec {
    let mut crates = BTreeMap::new();
    crates.insert("zcrate-1.0.0".to_string(), crate_spec("zcrate", "1.0.0"));
    crates.insert("acrate-2.0.0".to_string(), crate_spec("acrate", "2.0.0"));

    let mut tr_crates = BTreeMap::new();
    tr_crates.insert("zcrate-1.0.0".to_string(), edges());
    tr_crates.insert("acrate-2.0.0".to_string(), edges());

    let mut full: indexmap::IndexMap<String, TargetResolve> = indexmap::IndexMap::new();
    full.insert(
        "x86_64-unknown-linux-gnu".to_string(),
        TargetResolve { crates: tr_crates.clone() },
    );
    full.insert(
        "aarch64-apple-darwin".to_string(),
        TargetResolve { crates: tr_crates },
    );

    BuildSpec {
        version: gen_cargo::build_spec::SCHEMA_VERSION,
        workspace: WorkspaceSpec { members: Vec::new() },
        crates,
        root_crate: "zcrate-1.0.0".to_string(),
        workspace_members: vec!["zcrate-1.0.0".to_string()],
        flake_metadata: BTreeMap::new(),
        target_resolves: Some(gen_cargo::build_spec::CompactTargetResolves::from_full(full)),
        cargo_lock_sha256: Some("a".repeat(64)),
        // Deliberately reverse-sorted on insert: the emitted map must come
        // out canonical (BTreeMap key order) regardless, same discipline as
        // every other keyed map in the spec.
        manifest_sha256: [
            ("crates/z/Cargo.toml".to_string(), "b".repeat(64)),
            ("Cargo.toml".to_string(), "c".repeat(64)),
        ]
        .into_iter()
        .collect(),
    }
}

/// Adversarially permute every resolve-ordered Vec in a spec — reverse the
/// edge/feature lists — simulating a different platform's emission order.
/// (Maps are BTreeMap, so their order is construction-canonical regardless.)
fn shuffle(spec: &mut BuildSpec) {
    for c in spec.crates.values_mut() {
        c.runtime_dependencies.reverse();
        c.build_dependencies.reverse();
        c.features.reverse();
        for d in c.runtime_dependencies.iter_mut().chain(c.build_dependencies.iter_mut()) {
            d.features.reverse();
        }
    }
    if let Some(tr) = spec.target_resolves.as_mut() {
        let mut canon = |e: &mut CrateTargetEdges| {
            e.runtime_dependencies.reverse();
            e.build_dependencies.reverse();
            e.features.reverse();
            for d in e.runtime_dependencies.iter_mut().chain(e.build_dependencies.iter_mut()) {
                d.features.reverse();
            }
        };
        for e in tr.base.values_mut() {
            canon(e);
        }
        for ov in tr.targets.values_mut() {
            for e in ov.overrides.values_mut() {
                canon(e);
            }
        }
    }
}

fn json(spec: &BuildSpec) -> String {
    serde_json::to_string_pretty(spec).unwrap()
}

/// (1) Shuffle → canonicalize → BYTE-IDENTICAL to the unshuffled canonical
/// form. This IS the cross-platform proof: two hosts differ only by these
/// permutations, so permutation-invariance == platform-invariance.
#[test]
fn full_spec_is_byte_stable_under_input_permutation() {
    let mut base = fixture();
    base.canonicalize();
    let want = json(&base);

    let mut shuffled = fixture();
    shuffle(&mut shuffled);
    shuffled.canonicalize();
    assert_eq!(
        json(&shuffled),
        want,
        "canonicalized spec must be byte-identical regardless of input Vec order"
    );
}

/// (2) The slim delta inherits byte-stability: a delta distilled from a
/// shuffled-then-canonicalized spec equals one from the canonical spec.
#[test]
fn delta_is_byte_stable_under_input_permutation() {
    let mut base = fixture();
    base.canonicalize();
    let d_base = GenDelta::distill(&base).unwrap().to_json().unwrap();

    let mut shuffled = fixture();
    shuffle(&mut shuffled);
    shuffled.canonicalize();
    let d_shuffled = GenDelta::distill(&shuffled).unwrap().to_json().unwrap();

    assert_eq!(d_shuffled, d_base, "Cargo.gen.lock must be byte-stable cross-platform");
}

/// (3) Idempotence: canonicalize() applied twice == once.
#[test]
fn canonicalize_is_idempotent() {
    let mut once = fixture();
    once.canonicalize();
    let mut twice = once.clone();
    twice.canonicalize();
    assert_eq!(json(&once), json(&twice), "canonicalize must be idempotent");
}

/// (4) Content-preserving: the sort is a permutation of an unchanged multiset —
/// no edge/feature is dropped, added, or mutated, only reordered.
#[test]
fn canonicalize_preserves_content() {
    let mut spec = fixture();
    // Multiset of every edge package_key + every feature, BEFORE.
    let mut before: Vec<String> = Vec::new();
    for c in spec.crates.values() {
        for d in c.runtime_dependencies.iter().chain(c.build_dependencies.iter()) {
            before.push(d.package_key.clone());
            before.extend(d.features.iter().cloned());
        }
        before.extend(c.features.iter().cloned());
    }
    before.sort();

    spec.canonicalize();

    let mut after: Vec<String> = Vec::new();
    for c in spec.crates.values() {
        for d in c.runtime_dependencies.iter().chain(c.build_dependencies.iter()) {
            after.push(d.package_key.clone());
            after.extend(d.features.iter().cloned());
        }
        after.extend(c.features.iter().cloned());
    }
    after.sort();

    assert_eq!(before, after, "canonicalize must preserve the multiset of content");
}
