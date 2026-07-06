//! `go list` node-graph correctness on tiny fixtures — parse-level +
//! encoder edge-resolution (Go-I1 positive path).

mod common;

use gen_gomod::build_spec::{PackageKind, TargetTuple};
use gen_gomod::golist::parse_stream;
use gen_gomod::interp::apply;

// A stream carrying an ImportMap (vendor/replace rewrite): the writer's
// import path differs from the actual resolved package. Verified against
// the go1.25 `go list -json` field shape on 2026-07-06.
const REWRITE_STREAM: &str = r#"
{
	"Dir": "/w/cmd/app",
	"ImportPath": "example.com/app/cmd/app",
	"Name": "main",
	"Root": "/w",
	"GoFiles": ["main.go"],
	"Imports": ["example.com/app/pkg/util"],
	"ImportMap": {"example.com/app/pkg/util": "example.com/app/vendor/example.com/app/pkg/util"},
	"Module": {"Path": "example.com/app", "Main": true}
}
{
	"Dir": "/w/vendor/example.com/app/pkg/util",
	"ImportPath": "example.com/app/vendor/example.com/app/pkg/util",
	"Name": "util",
	"Root": "/w",
	"DepOnly": true,
	"GoFiles": ["util.go"],
	"Imports": []
}
"#;

#[test]
fn parse_classifies_kind_and_dep_only() {
    let pkgs = parse_stream(REWRITE_STREAM).expect("parse");
    assert_eq!(pkgs.len(), 2);
    assert_eq!(pkgs[0].kind(), PackageKind::Main);
    assert!(!pkgs[0].dep_only);
    // ImportMap captured.
    assert_eq!(
        pkgs[0].import_map.get("example.com/app/pkg/util").map(String::as_str),
        Some("example.com/app/vendor/example.com/app/pkg/util")
    );
    assert_eq!(pkgs[1].kind(), PackageKind::Module);
    assert!(pkgs[1].dep_only);
}

#[test]
fn encoder_resolves_import_edge_through_import_map() {
    use gen_gomod::interp::EncodeCtx;
    use gen_gomod::testkit::MockGoBuildEnv;
    let tuple = TargetTuple::new("linux", "amd64", vec![]);
    let env = MockGoBuildEnv::new()
        .with_list(&tuple, REWRITE_STREAM.trim())
        .with_file("/w/go.mod", "module example.com/app\n\ngo 1.25\n")
        .with_file("/w/cmd/app/main.go", "package main\nfunc main() {}\n")
        .with_file("/w/vendor/example.com/app/pkg/util/util.go", "package util\n");
    let spec = apply(&env, &EncodeCtx { root: common::ROOT.into(), tuple }).expect("encode");

    // The main's edge is rewritten through ImportMap to the vendored
    // actual package's node key — and that key exists (Go-I1 holds).
    let app = &spec.packages["example.com/app/cmd/app#linux-amd64"];
    let vendored_key = "example.com/app/vendor/example.com/app/pkg/util#linux-amd64";
    assert!(
        app.imports.contains(&vendored_key.to_string()),
        "edge must resolve through ImportMap to the vendored node; got {:?}",
        app.imports
    );
    assert!(spec.packages.contains_key(vendored_key));
    // and the main carries the rewrite for the interpreter's importcfg.
    assert!(app.import_map.contains_key("example.com/app/pkg/util"));
}

#[test]
fn gate_a_edges_all_resolve() {
    let tuple = TargetTuple::new("linux", "amd64", vec![]);
    let (env, ctx) = common::gate_a("v1", &tuple);
    let spec = apply(&env, &ctx).expect("encode");
    // every import edge names a node present in the graph.
    for (key, p) in &spec.packages {
        for edge in &p.imports {
            assert!(spec.packages.contains_key(edge), "node `{key}` edge `{edge}` unresolved");
        }
    }
}
