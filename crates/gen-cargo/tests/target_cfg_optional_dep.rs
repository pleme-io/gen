//! Regression test for the target-cfg-gated optional-dependency fix
//! (cid fleet-rebuild wedge, 2026-06-17).
//!
//! rustix's libc backend (`libc` + `libc_errno`) is declared
//! `optional = true` but is pulled PURELY by a
//! `[target.'cfg(all(not(windows), any(rustix_use_libc, miri,
//! not(all(target_os = "linux", ...)))))'.dependencies]` match on
//! apple/bsd — NO `use-libc` feature appears in the resolved feature
//! set. gen-cargo's unactivated-optional filter is feature-only, so it
//! must TRUST cargo's `--filter-platform` resolve for target-gated
//! optionals (`declared.target.is_some()`) instead of re-deriving
//! activation from features. Dropping these edges produced
//! `error[E0432]: unresolved import libc` on every darwin build and
//! wedged the whole fleet rebuild.
//!
//! Runs on any host: `--filter-platform aarch64-apple-darwin` is a
//! resolve-time filter, so cargo evaluates the apple cfg even on linux.
//! Skips gracefully when cargo can't reach a registry (hermetic CI).

use std::fs;
use std::process::Command;
use tempfile::TempDir;

fn write_rustix_fixture() -> TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    // rustix with the `fs` feature exercises the libc backend on apple.
    fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "rustix-fixture"
version = "0.1.0"
edition = "2021"

[dependencies]
rustix = { version = "1", features = ["fs"] }
"#,
    )
    .unwrap();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/lib.rs"), "pub fn f() {}\n").unwrap();
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

#[test]
fn rustix_libc_backend_survives_optional_filter_on_apple() {
    let dir = write_rustix_fixture();
    if !dir.path().join("Cargo.lock").exists() {
        eprintln!("skipping: no Cargo.lock could be generated (offline, no registry cache)");
        return;
    }

    let spec = match gen_cargo::build_spec::generate_for_target(dir.path(), "aarch64-apple-darwin") {
        Ok(s) => s,
        Err(e) => {
            eprintln!("skipping: generate_for_target failed (offline metadata?): {e}");
            return;
        }
    };

    let rustix = spec
        .crates
        .values()
        .find(|c| c.name == "rustix")
        .expect("rustix must appear in the apple-target resolve");

    let dep_names: Vec<&str> = rustix
        .runtime_dependencies
        .iter()
        .map(|d| d.name.as_str())
        .collect();

    // The exact regression: these target-cfg-gated optional deps were
    // dropped by the feature-only filter. If either is gone, the bug
    // is back and darwin builds will fail with `unresolved import libc`.
    assert!(
        dep_names.iter().any(|n| *n == "libc"),
        "rustix's target-cfg-gated optional `libc` was dropped on apple \
         (the optional filter must trust cargo's --filter-platform resolve \
         for target-scoped deps). deps = {dep_names:?}"
    );
    assert!(
        dep_names.iter().any(|n| *n == "libc_errno" || *n == "errno"),
        "rustix's target-cfg-gated optional `errno`/`libc_errno` was dropped \
         on apple. deps = {dep_names:?}"
    );

    // And the surviving edge is target-scoped — that's the predicate the
    // fix keys on (`declared.target.is_some()`), distinguishing it from a
    // feature-gated optional like sqlx-sqlite (which must stay filtered).
    let libc_edge = rustix
        .runtime_dependencies
        .iter()
        .find(|d| d.name == "libc")
        .unwrap();
    assert!(
        libc_edge.target.is_some(),
        "the surviving libc edge must be target-cfg-scoped; \
         got target = {:?}",
        libc_edge.target
    );
}
