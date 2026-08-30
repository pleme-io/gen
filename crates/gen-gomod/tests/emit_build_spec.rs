//! `gen build .` emit path — proves that encoding a Go module through
//! the gomod adapter writes a valid `Go.build-spec.json` next to
//! `go.mod`. This is the gomod peer of gen-cargo's build-spec emission
//! and the load-bearing half of the CLI `gen build` gomod arm.
//!
//! The `go list` + source reads stay behind the [`MockGoBuildEnv`] seam
//! (TYPED-SPEC + INTERPRETER TRIPLET) — no `go` toolchain, no source
//! filesystem reads — while the OUTPUT write goes to a real temp
//! directory so the actual `std::fs::write` in `emit::generate_with_env`
//! is exercised. Always-green; the `#[ignore]` end-to-end test that
//! shells the real `gen` binary lives in `real_go.rs`.

use std::fs;
use std::path::PathBuf;

use gen_gomod::build_spec::{Renderer, TargetTuple};
use gen_gomod::emit::{SPEC_FILENAME, generate_with_env};
use gen_gomod::testkit::MockGoBuildEnv;
use serde_json::{Value, json};

/// A distinct writable temp root per test invocation.
fn temp_root(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "gen-gomod-emit-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("mk temp root");
    dir
}

fn stream(objs: &[Value]) -> String {
    objs.iter()
        .map(|o| serde_json::to_string(o).unwrap())
        .collect::<Vec<_>>()
        .join("\n")
}

fn std_pkg(import_path: &str, go_file: &str) -> Value {
    json!({
        "Dir": format!("/goroot/src/{import_path}"),
        "ImportPath": import_path,
        "Name": import_path.rsplit('/').next().unwrap(),
        "Root": "/goroot/src",
        "Standard": true,
        "DepOnly": true,
        "GoFiles": [go_file],
        "Imports": []
    })
}

/// Build a filesystem-free `go list` mock for a single-main module
/// (`cmd/tool` importing only `fmt`) rooted at `root`, plus the source
/// reads the encoder will make — all keyed under the real temp `root`.
fn dep_free_env(root: &std::path::Path, tuple: &TargetTuple) -> MockGoBuildEnv {
    let root_s = root.to_string_lossy().to_string();
    let main = json!({
        "Dir": format!("{root_s}/cmd/tool"),
        "ImportPath": "example.com/tool/cmd/tool",
        "Name": "main",
        "Root": root_s,
        "GoFiles": ["main.go"],
        "Imports": ["fmt"],
        "Module": {"Path": "example.com/tool", "Main": true}
    });
    let objs = [main, std_pkg("fmt", "print.go")];
    MockGoBuildEnv::new()
        .with_list(tuple, stream(&objs))
        .with_file(
            format!("{root_s}/go.mod"),
            "module example.com/tool\n\ngo 1.25\n",
        )
        .with_file(
            format!("{root_s}/cmd/tool/main.go"),
            "package main\nfunc main() {}\n",
        )
}

/// The core proof: `gen build .` on a Go module emits `Go.build-spec.json`.
#[test]
fn gen_build_emits_go_build_spec_json() {
    let root = temp_root("core");
    let tuple = TargetTuple::new("linux", "amd64", vec![]);
    let env = dep_free_env(&root, &tuple);

    let out = generate_with_env(&env, &root, tuple).expect("emit Go.build-spec.json");

    // 1. The file exists, at the canonical name, next to go.mod.
    assert_eq!(out, root.join(SPEC_FILENAME));
    assert_eq!(out.file_name().unwrap(), "Go.build-spec.json");
    assert!(out.is_file(), "Go.build-spec.json must be written");

    // 2. It is valid JSON that round-trips to a v2 incremental BuildSpec.
    let body = fs::read_to_string(&out).expect("read emitted spec");
    assert!(body.ends_with('\n'), "canonical trailing newline");
    let spec: gen_gomod::BuildSpec =
        serde_json::from_str(&body).expect("emitted JSON parses as BuildSpec");
    assert_eq!(spec.version, gen_gomod::SCHEMA_VERSION);
    assert!(matches!(spec.renderer, Renderer::Incremental));

    // 3. The graph is real: the main node + its std/fmt dep are present,
    //    keyed at the target tuple.
    assert_eq!(spec.root_package, "example.com/tool/cmd/tool#linux-amd64");
    assert!(
        spec.packages
            .contains_key("example.com/tool/cmd/tool#linux-amd64")
    );
    assert!(spec.packages.contains_key("std/fmt#linux-amd64"));

    // 4. The emitted spec satisfies its own invariants (0 violations).
    assert!(
        gen_gomod::invariants::check(&spec).is_empty(),
        "emitted spec must hold all Go invariants"
    );

    let _ = fs::remove_dir_all(&root);
}

/// The write is deterministic: two emits of the same module produce a
/// byte-identical `Go.build-spec.json`.
#[test]
fn emit_is_byte_deterministic() {
    let tuple = TargetTuple::new("linux", "amd64", vec![]);

    let root_a = temp_root("det-a");
    let out_a =
        generate_with_env(&dep_free_env(&root_a, &tuple), &root_a, tuple.clone()).expect("emit a");
    // Read then normalize out the (identical) root path — the spec body
    // carries no absolute paths (relative_path is under the module root),
    // so the two bodies are directly comparable.
    let body_a = fs::read_to_string(&out_a).expect("read a");

    let root_b = temp_root("det-b");
    let out_b = generate_with_env(&dep_free_env(&root_b, &tuple), &root_b, tuple).expect("emit b");
    let body_b = fs::read_to_string(&out_b).expect("read b");

    assert_eq!(
        body_a, body_b,
        "same module ⇒ byte-identical Go.build-spec.json"
    );

    let _ = fs::remove_dir_all(&root_a);
    let _ = fs::remove_dir_all(&root_b);
}
