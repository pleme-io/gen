//! Encoder phase tests — `interp::apply` driven entirely by
//! [`MockGoBuildEnv`] (no `go`, no filesystem). Each test pins one
//! conditional-emission rule from the M1 build doc §6 (the encoder-side
//! half of the GEN TYPED-SPEC CONTRACT).

mod common;

use gen_gomod::build_spec::{PackageKind, PackageSource, Renderer, TargetTuple};
use gen_gomod::interp::apply;
use gen_gomod::invariants;

fn linux() -> TargetTuple {
    TargetTuple::new("linux", "amd64", vec![])
}

// ── the internal-shared graph: nodes, roots, dedup ───────────────────
#[test]
fn encodes_internal_shared_graph() {
    let (env, ctx) = common::gate_a("v1", &linux());
    let spec = apply(&env, &ctx).expect("encode");

    assert!(spec.renderer.is_incremental());
    assert_eq!(spec.module.module_path, "example.com/fix");
    assert_eq!(spec.module.go_version, "1.25");

    // 3 module/main nodes + 2 std nodes.
    assert_eq!(spec.packages.len(), 5, "greet+hello+bye+fmt+embed");

    let greet = "example.com/fix/internal/greet#linux-amd64";
    let hello = "example.com/fix/cmd/hello#linux-amd64";
    let bye = "example.com/fix/cmd/bye#linux-amd64";
    assert!(spec.packages.contains_key(greet));
    assert!(spec.packages.contains_key(hello));
    assert!(spec.packages.contains_key(bye));
    assert!(spec.packages.contains_key("std/fmt#linux-amd64"));
    assert!(spec.packages.contains_key("std/embed#linux-amd64"));

    // root = smallest main (bye < hello); members = both mains, sorted.
    assert_eq!(spec.root_package, bye);
    assert_eq!(spec.workspace_members, vec![bye.to_string(), hello.to_string()]);
    assert_eq!(spec.packages[greet].kind, PackageKind::Module);
    assert_eq!(spec.packages[hello].kind, PackageKind::Main);
}

// ── Gate A: the shared internal package is ONE node both mains reuse ──
#[test]
fn shared_internal_compiles_once() {
    let (env, ctx) = common::gate_a("v1", &linux());
    let spec = apply(&env, &ctx).expect("encode");
    let greet = "example.com/fix/internal/greet#linux-amd64";
    // exactly one greet node; both mains name that same key as an import.
    assert!(spec.packages.contains_key(greet));
    for main in ["example.com/fix/cmd/hello#linux-amd64", "example.com/fix/cmd/bye#linux-amd64"] {
        assert!(
            spec.packages[main].imports.contains(&greet.to_string()),
            "{main} must import the single shared greet node"
        );
    }
}

// ── Gate A incremental proof: editing the shared source changes ONLY
//    its node's source_hash; the two mains' hashes are untouched. ──────
#[test]
fn editing_shared_source_changes_only_its_hash() {
    let (env1, ctx1) = common::gate_a("BANNER v1", &linux());
    let (env2, ctx2) = common::gate_a("BANNER v2 EDITED", &linux());
    let a = apply(&env1, &ctx1).expect("encode v1");
    let b = apply(&env2, &ctx2).expect("encode v2");

    let greet = "example.com/fix/internal/greet#linux-amd64";
    let hello = "example.com/fix/cmd/hello#linux-amd64";
    let bye = "example.com/fix/cmd/bye#linux-amd64";

    // greet's embed content changed → its content address changed.
    assert_ne!(
        a.packages[greet].source_hash, b.packages[greet].source_hash,
        "editing greet's embed must change greet's source_hash"
    );
    // the mains' sources did NOT change → identical hashes (store hits).
    assert_eq!(a.packages[hello].source_hash, b.packages[hello].source_hash);
    assert_eq!(a.packages[bye].source_hash, b.packages[bye].source_hash);
}

// ── Go-I9: embed patterns + files captured ───────────────────────────
#[test]
fn embed_is_captured() {
    let (env, ctx) = common::gate_a("v1", &linux());
    let spec = apply(&env, &ctx).expect("encode");
    let greet = &spec.packages["example.com/fix/internal/greet#linux-amd64"];
    assert_eq!(greet.embed.patterns, vec!["banner.txt"]);
    assert_eq!(greet.embed.files, vec!["banner.txt"]);
}

// ── Go-I6: build constraints resolved per-tuple by the encoder ───────
#[test]
fn build_constraints_resolved_per_tuple() {
    let (le, lc) = common::gate_a("v1", &TargetTuple::new("linux", "amd64", vec![]));
    let (de, dc) = common::gate_a("v1", &TargetTuple::new("darwin", "arm64", vec![]));
    let lin = apply(&le, &lc).expect("linux");
    let dar = apply(&de, &dc).expect("darwin");
    let lg = &lin.packages["example.com/fix/internal/greet#linux-amd64"];
    let dg = &dar.packages["example.com/fix/internal/greet#darwin-arm64"];
    assert!(lg.go_files.contains(&"greet_linux.go".to_string()));
    assert!(!lg.go_files.contains(&"greet_darwin.go".to_string()));
    assert!(dg.go_files.contains(&"greet_darwin.go".to_string()));
    assert!(!dg.go_files.contains(&"greet_linux.go".to_string()));
}

// ── Go-I10: std nodes carry Std source, no per-node hash; Target tree ─
#[test]
fn std_nodes_are_std_source_and_unhashed() {
    let (env, ctx) = common::gate_a("v1", &linux());
    let spec = apply(&env, &ctx).expect("encode");
    let fmtn = &spec.packages["std/fmt#linux-amd64"];
    assert_eq!(fmtn.kind, PackageKind::Std);
    assert!(matches!(fmtn.source, PackageSource::Std));
    assert!(fmtn.source_hash.is_empty(), "std content-addressed by the std-tree, not per-file");
    assert!(fmtn.tree.is_target());
}

// ── Go-I3: vendored + in-tree replace relative_paths ─────────────────
#[test]
fn vendored_and_replace_relative_paths() {
    let (env, ctx) = common::vendored_and_replace(&linux());
    let spec = apply(&env, &ctx).expect("encode");
    let vend = &spec.packages["github.com/foo/bar#linux-amd64"];
    match &vend.source {
        PackageSource::Vendored { relative_path } => assert_eq!(relative_path, "vendor/github.com/foo/bar"),
        s => panic!("expected vendored source, got {s:?}"),
    }
    // the replaced module resolves its Dir to the in-tree replacement.
    let repl = &spec.packages["github.com/example/sdk-go/v3#linux-amd64"];
    match &repl.source {
        PackageSource::Vendored { relative_path } => {
            assert_eq!(relative_path, "go/src/client/sdktest/sdk-go");
        }
        s => panic!("expected vendored source, got {s:?}"),
    }
    // provenance carries the vendored dep's module version.
    assert_eq!(vend.module.as_ref().and_then(|m| m.version.as_deref()), Some("v1.2.3"));
}

// ── Go-I12: a cgo node is rejected at encode time (fail closed) ──────
#[test]
fn cgo_node_is_rejected() {
    let (env, ctx) = common::gate_a_with_cgo(&linux());
    let err = apply(&env, &ctx).expect_err("cgo subgraph must be rejected in M1");
    assert!(
        matches!(&err, gen_gomod::GomodError::Interp { phase: "reject-cgo", .. }),
        "expected reject-cgo, got {err:?}"
    );
}

// ── Go-I7: go_sum tie is the empty-string hash when dep-free ─────────
#[test]
fn dep_free_go_sum_tie_is_empty_hash() {
    let (env, ctx) = common::dep_free(&linux());
    let spec = apply(&env, &ctx).expect("encode");
    assert!(!spec.module.has_external_deps);
    assert_eq!(
        spec.go_sum_sha256.as_deref(),
        Some("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
    );
}

// ── the encoded spec always satisfies its own invariants ─────────────
#[test]
fn encoded_specs_satisfy_invariants() {
    for (env, ctx) in [common::gate_a("v1", &linux()).0, common::dep_free(&linux()).0]
        .into_iter()
        .zip([common::gate_a("v1", &linux()).1, common::dep_free(&linux()).1])
    {
        let spec = apply(&env, &ctx).expect("encode");
        let v = invariants::check(&spec);
        assert!(v.is_empty(), "encoded spec violated invariants: {v:?}");
    }
    let (ve, vc) = common::vendored_and_replace(&linux());
    let vs = apply(&ve, &vc).expect("encode");
    assert!(invariants::check(&vs).is_empty());
}

// ── target_resolves round-trips (single tuple; M-multitarget-ready) ──
#[test]
fn target_resolves_round_trip() {
    let (env, ctx) = common::gate_a("v1", &linux());
    let spec = apply(&env, &ctx).expect("encode");
    let tr = spec.target_resolves.expect("target_resolves populated");
    let expanded = tr.expand();
    assert!(expanded.contains_key("#linux-amd64"));
    // greet's edges are present in the resolve for the tuple.
    let greet = "example.com/fix/internal/greet#linux-amd64";
    let edges = &expanded["#linux-amd64"].packages[greet];
    assert!(edges.imports.contains(&"std/fmt#linux-amd64".to_string()));
    assert!(edges.imports.contains(&"std/embed#linux-amd64".to_string()));
}

// ── serialized spec is deterministic (canonical BTreeMap key order) ──
#[test]
fn serialized_spec_is_byte_stable() {
    let (env, ctx) = common::gate_a("v1", &linux());
    let a = serde_json::to_string(&apply(&env, &ctx).unwrap()).unwrap();
    let b = serde_json::to_string(&apply(&env, &ctx).unwrap()).unwrap();
    assert_eq!(a, b, "two encodes of identical input must be byte-identical");
    // and the payload round-trips.
    let back: gen_gomod::build_spec::BuildSpec = serde_json::from_str(&a).unwrap();
    assert!(matches!(back.renderer, Renderer::Incremental));
}
