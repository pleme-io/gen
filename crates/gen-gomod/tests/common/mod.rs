//! Shared M1 fixtures — filesystem-free `go list` stubs driving the
//! encoder through [`MockGoBuildEnv`]. These are the canonical Gate-A
//! shapes (2 mains sharing one internal pure-Go+embed package, std-only
//! deps) + a dep-free single-main, built from `serde_json` so there are
//! no hand-format escaping bugs.

#![allow(dead_code)]

use gen_gomod::build_spec::TargetTuple;
use gen_gomod::interp::EncodeCtx;
use gen_gomod::testkit::MockGoBuildEnv;
use serde_json::{json, Value};

pub const ROOT: &str = "/w";

/// Serialize a Vec of package objects into the concatenated-object
/// stream `go list -json` emits (objects separated by newlines).
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

/// The Gate-A fixture: `cmd/hello` + `cmd/bye`, both importing the shared
/// `internal/greet` (which has a `//go:embed banner.txt`), std deps
/// `fmt` + `embed`. `banner` parameterizes the embed content so an "edit"
/// can be simulated; `tuple` selects which platform file `greet` carries.
pub fn gate_a(banner: &str, tuple: &TargetTuple) -> (MockGoBuildEnv, EncodeCtx) {
    let plat_file = format!("greet_{}.go", tuple.goos);
    let greet = json!({
        "Dir": "/w/internal/greet",
        "ImportPath": "example.com/fix/internal/greet",
        "Name": "greet",
        "Root": "/w",
        "DepOnly": true,
        "GoFiles": ["greet.go", plat_file],
        "EmbedPatterns": ["banner.txt"],
        "EmbedFiles": ["banner.txt"],
        "Imports": ["embed", "fmt"],
        "Module": {"Path": "example.com/fix", "Main": true, "GoVersion": "1.25"}
    });
    let hello = json!({
        "Dir": "/w/cmd/hello",
        "ImportPath": "example.com/fix/cmd/hello",
        "Name": "main",
        "Root": "/w",
        "GoFiles": ["main.go"],
        "Imports": ["example.com/fix/internal/greet"],
        "Module": {"Path": "example.com/fix", "Main": true}
    });
    let bye = json!({
        "Dir": "/w/cmd/bye",
        "ImportPath": "example.com/fix/cmd/bye",
        "Name": "main",
        "Root": "/w",
        "GoFiles": ["main.go"],
        "Imports": ["example.com/fix/internal/greet"],
        "Module": {"Path": "example.com/fix", "Main": true}
    });
    let objs = [greet, hello, bye, std_pkg("fmt", "print.go"), std_pkg("embed", "embed.go")];

    let env = MockGoBuildEnv::new()
        .with_list(tuple, stream(&objs))
        .with_file("/w/go.mod", "module example.com/fix\n\ngo 1.25\n")
        .with_file("/w/internal/greet/greet.go", "package greet\n")
        .with_file("/w/internal/greet/greet_linux.go", "package greet\nconst P = \"linux\"\n")
        .with_file("/w/internal/greet/greet_darwin.go", "package greet\nconst P = \"darwin\"\n")
        .with_file("/w/internal/greet/banner.txt", banner)
        .with_file("/w/cmd/hello/main.go", "package main\nfunc main() {}\n")
        .with_file("/w/cmd/bye/main.go", "package main\nfunc main() {}\n");
    (env, EncodeCtx { root: ROOT.into(), tuple: tuple.clone() })
}

/// A dep-free single-main module (`cmd/tool` importing only `fmt`).
pub fn dep_free(tuple: &TargetTuple) -> (MockGoBuildEnv, EncodeCtx) {
    let main = json!({
        "Dir": "/w/cmd/tool",
        "ImportPath": "example.com/tool/cmd/tool",
        "Name": "main",
        "Root": "/w",
        "GoFiles": ["main.go"],
        "Imports": ["fmt"],
        "Module": {"Path": "example.com/tool", "Main": true}
    });
    let objs = [main, std_pkg("fmt", "print.go")];
    let env = MockGoBuildEnv::new()
        .with_list(tuple, stream(&objs))
        .with_file("/w/go.mod", "module example.com/tool\n\ngo 1.25\n")
        .with_file("/w/cmd/tool/main.go", "package main\nfunc main() {}\n");
    (env, EncodeCtx { root: ROOT.into(), tuple: tuple.clone() })
}

/// A fixture exercising Go-I3 for BOTH a vendored dep (`vendor/…`) and an
/// in-tree filesystem `replace` (`go/src/client/…`) — the large service
/// monorepo shape.
pub fn vendored_and_replace(tuple: &TargetTuple) -> (MockGoBuildEnv, EncodeCtx) {
    let main = json!({
        "Dir": "/w/go/src/microservices/service-a",
        "ImportPath": "example.com/monorepo/go/src/microservices/service-a",
        "Name": "main",
        "Root": "/w",
        "GoFiles": ["main.go"],
        "Imports": ["github.com/foo/bar", "github.com/example/sdk-go/v3"],
        "Module": {"Path": "example.com/monorepo", "Main": true}
    });
    // Vendored dep — Dir under /w/vendor/…, ImportMap identity omitted.
    let vendored = json!({
        "Dir": "/w/vendor/github.com/foo/bar",
        "ImportPath": "github.com/foo/bar",
        "Name": "bar",
        "Root": "/w",
        "DepOnly": true,
        "GoFiles": ["bar.go"],
        "Imports": [],
        "Module": {"Path": "github.com/foo/bar", "Version": "v1.2.3"}
    });
    // In-tree filesystem replace — Dir points at the replacement under go/src.
    let replaced = json!({
        "Dir": "/w/go/src/client/sdktest/sdk-go",
        "ImportPath": "github.com/example/sdk-go/v3",
        "Name": "sdkgo",
        "Root": "/w",
        "DepOnly": true,
        "GoFiles": ["client.go"],
        "Imports": [],
        "Module": {"Path": "example.com/monorepo", "Main": true}
    });
    let objs = [main, vendored, replaced];
    let env = MockGoBuildEnv::new()
        .with_list(tuple, stream(&objs))
        .with_file("/w/go.mod", "module example.com/monorepo\n\ngo 1.26\n")
        .with_file("/w/go/src/microservices/service-a/main.go", "package main\nfunc main() {}\n")
        .with_file("/w/vendor/github.com/foo/bar/bar.go", "package bar\n")
        .with_file("/w/go/src/client/sdktest/sdk-go/client.go", "package sdkgo\n");
    (env, EncodeCtx { root: ROOT.into(), tuple: tuple.clone() })
}

/// Gate-A but the shared `greet` package has a cgo file — used to prove
/// Go-I12 rejection at encode time.
pub fn gate_a_with_cgo(tuple: &TargetTuple) -> (MockGoBuildEnv, EncodeCtx) {
    let (mut env, ctx) = gate_a("v1", tuple);
    // Replace greet's list object with one carrying CgoFiles.
    let greet = json!({
        "Dir": "/w/internal/greet",
        "ImportPath": "example.com/fix/internal/greet",
        "Name": "greet",
        "Root": "/w",
        "DepOnly": true,
        "GoFiles": ["greet.go"],
        "CgoFiles": ["cgo.go"],
        "Imports": ["fmt"],
        "Module": {"Path": "example.com/fix", "Main": true}
    });
    let hello = json!({
        "Dir": "/w/cmd/hello","ImportPath": "example.com/fix/cmd/hello","Name": "main","Root": "/w",
        "GoFiles": ["main.go"],"Imports": ["example.com/fix/internal/greet"],
        "Module": {"Path": "example.com/fix", "Main": true}
    });
    let objs = [greet, hello, std_pkg("fmt", "print.go")];
    env = env.with_list(tuple, stream(&objs));
    (env, ctx)
}
