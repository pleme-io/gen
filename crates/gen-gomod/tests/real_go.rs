//! End-to-end smoke test against the REAL `go` toolchain — grounds
//! `RealGoBuildEnv` (the actual subprocess + filesystem path) beyond the
//! mocked unit tests. `#[ignore]` by default: it needs `go` on PATH and
//! writes a temp module. Run explicitly:
//!
//! ```text
//! cargo test -p gen-gomod --test real_go -- --ignored
//! ```

use std::fs;
use std::path::PathBuf;

use gen_gomod::build_spec::{PackageKind, TargetTuple};
use gen_gomod::interp::{apply, EncodeCtx, RealGoBuildEnv};
use gen_gomod::invariants;

/// Build the Gate-A fixture on disk (2 mains sharing a pure-Go+embed
/// internal package, std-only deps), run the encoder through the real
/// `go list`, assert the incremental graph + Gate-A dedup.
#[test]
#[ignore = "needs `go` on PATH; run with --ignored"]
fn gate_a_encodes_with_real_go() {
    let dir: PathBuf = std::env::temp_dir().join(format!("gen-gomod-real-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    for sub in ["internal/greet", "cmd/hello", "cmd/bye"] {
        fs::create_dir_all(dir.join(sub)).unwrap();
    }
    fs::write(dir.join("go.mod"), "module example.com/fix\n\ngo 1.25\n").unwrap();
    fs::write(
        dir.join("internal/greet/greet.go"),
        "package greet\n\nimport (\n\t_ \"embed\"\n\t\"fmt\"\n)\n\n//go:embed banner.txt\nvar Banner string\n\nfunc Hello(n string) string { return fmt.Sprintf(\"hi %s %s\", n, Banner) }\nfunc Bye(n string) string   { return fmt.Sprintf(\"bye %s\", n) }\n",
    )
    .unwrap();
    fs::write(dir.join("internal/greet/banner.txt"), "BANNER\n").unwrap();
    fs::write(
        dir.join("cmd/hello/main.go"),
        "package main\n\nimport \"example.com/fix/internal/greet\"\n\nfunc main() { println(greet.Hello(\"w\")) }\n",
    )
    .unwrap();
    fs::write(
        dir.join("cmd/bye/main.go"),
        "package main\n\nimport \"example.com/fix/internal/greet\"\n\nfunc main() { println(greet.Bye(\"w\")) }\n",
    )
    .unwrap();

    // Dep-free ⇒ no vendor dir needed; use -mod=mod for this smoke test.
    let env = RealGoBuildEnv { mod_mode: "mod".to_string() };
    let tuple = TargetTuple::host();
    let spec = apply(&env, &EncodeCtx { root: dir.clone(), tuple: tuple.clone() })
        .expect("real go list encode");

    let greet = format!("example.com/fix/internal/greet{}", tuple.suffix());
    let hello = format!("example.com/fix/cmd/hello{}", tuple.suffix());
    let bye = format!("example.com/fix/cmd/bye{}", tuple.suffix());

    assert!(spec.renderer.is_incremental());
    assert!(spec.packages.contains_key(&greet), "greet node present");
    assert_eq!(spec.packages[&greet].kind, PackageKind::Module);
    assert!(!spec.packages[&greet].embed.files.is_empty(), "embed captured");
    // both mains share the ONE greet node.
    assert!(spec.packages[&hello].imports.contains(&greet));
    assert!(spec.packages[&bye].imports.contains(&greet));
    // real std closure is present + keyed under std/.
    assert!(spec.packages.keys().any(|k| k.starts_with("std/fmt")));
    // the encoded real-graph satisfies its own invariants.
    assert!(invariants::check(&spec).is_empty(), "real graph invariants");

    let _ = fs::remove_dir_all(&dir);
}
