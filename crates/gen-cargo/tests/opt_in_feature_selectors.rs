//! Regression test for the missing `--features` pass-through bug
//! (`gen/crates/gen-cargo/src/build_spec.rs::generate_for_target`,
//! surfaced 2026-07-23 via `breathe-retirada-ci`'s optional `watch-cli`
//! feature, which gates its `retirada-ci-watch` `[[bin]]` behind
//! `required-features` — before this fix, the deps that feature gates
//! (octocrab/tokio/clap/tracing-subscriber) could NEVER enter the
//! emitted `Cargo.build-spec.json`, no matter what the crate declared,
//! because `MetadataCommand` never told cargo's resolver about any
//! non-default feature).
//!
//! The fix is deliberately NOT `CargoOpt::AllFeatures` — this file's
//! own `resolve_pkg_source`/phantom-optional-dep-pruning logic is
//! hardened against exactly that regression (the documented
//! `sqlx-sqlite` precedent: blanket-activating optional deps drags in
//! drivers that were never meant to build and breaks the musl image
//! build). The fix is OPT-IN and PACKAGE-QUALIFIED: a workspace member
//! declares `[package.metadata.pleme].features = ["some-feature"]` in
//! its own `Cargo.toml`, and `opt_in_feature_selectors` turns that into
//! a `CargoOpt::SomeFeatures(["<pkg>/<feature>", ...])` — activating
//! ONLY what was explicitly asked for, for ONLY that crate.
//!
//! Runs fully offline: every dependency in the fixture is a same-tree
//! path dependency, so `cargo generate-lockfile` never touches the
//! network or a registry index.

use std::fs;
use std::process::Command;

use gen_cargo::build_spec;
use tempfile::TempDir;

/// A tiny two-crate fixture: `opt-in-fixture` optionally depends (path,
/// same tree) on `helper`, gated by feature `extra`. `enable_opt_in`
/// controls whether `opt-in-fixture`'s own `[package.metadata.pleme]`
/// asks gen to activate `extra`.
fn write_fixture(enable_opt_in: bool) -> TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();

    let opt_in_block = if enable_opt_in {
        "\n[package.metadata.pleme]\nfeatures = [\"extra\"]\n"
    } else {
        ""
    };

    fs::write(
        root.join("Cargo.toml"),
        format!(
            r#"[package]
name = "opt-in-fixture"
version = "0.1.0"
edition = "2021"

[dependencies]
helper = {{ path = "helper", optional = true }}

[features]
extra = ["dep:helper"]
{opt_in_block}"#
        ),
    )
    .unwrap();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/lib.rs"), "pub fn f() {}\n").unwrap();

    fs::create_dir_all(root.join("helper/src")).unwrap();
    fs::write(
        root.join("helper/Cargo.toml"),
        r#"[package]
name = "helper"
version = "0.1.0"
edition = "2021"
"#,
    )
    .unwrap();
    fs::write(root.join("helper/src/lib.rs"), "pub fn h() {}\n").unwrap();

    // Fully local (path-only) deps — never touches the network.
    let out = Command::new("cargo")
        .args(["generate-lockfile", "--offline"])
        .current_dir(root)
        .output()
        .expect("run cargo generate-lockfile");
    assert!(
        out.status.success(),
        "cargo generate-lockfile failed (should be fully offline for a \
         path-only-dep fixture): {}",
        String::from_utf8_lossy(&out.stderr)
    );

    dir
}

fn host_triple() -> &'static str {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "aarch64-apple-darwin"
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        "x86_64-apple-darwin"
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "x86_64-unknown-linux-gnu"
    } else {
        "aarch64-unknown-linux-gnu"
    }
}

#[test]
fn without_opt_in_the_gated_optional_dep_is_pruned() {
    let dir = write_fixture(false);
    let spec = build_spec::generate_for_target(dir.path(), host_triple())
        .expect("spec generation (offline, path-only fixture)");

    let main = spec
        .crates
        .values()
        .find(|c| c.name == "opt-in-fixture")
        .expect("opt-in-fixture must appear in the resolve");

    // This is TODAY's (pre-fix and post-fix, unchanged) correct
    // behavior for a crate that does NOT opt in: `extra` is not a
    // default feature, so cargo never activates `helper`, and the
    // phantom-optional-dep filter prunes the edge — exactly matching
    // what a plain `cargo build` (no flags) would produce.
    assert!(
        main.runtime_dependencies.iter().all(|d| d.name != "helper"),
        "helper must NOT be a runtime dependency of opt-in-fixture when \
         `extra` isn't activated — got {:?}",
        main.runtime_dependencies
            .iter()
            .map(|d| d.name.as_str())
            .collect::<Vec<_>>()
    );
}

#[test]
fn opt_in_activates_the_gated_optional_dep() {
    let dir = write_fixture(true);
    let spec = build_spec::generate_for_target(dir.path(), host_triple())
        .expect("spec generation (offline, path-only fixture)");

    let main = spec
        .crates
        .values()
        .find(|c| c.name == "opt-in-fixture")
        .expect("opt-in-fixture must appear in the resolve");

    // THE FIX: `[package.metadata.pleme].features = ["extra"]` makes
    // gen pass `--features opt-in-fixture/extra` to cargo's resolver,
    // so `helper` (gated by `dep:helper` inside `extra`) is now a real
    // activated edge and appears in the emitted spec.
    assert!(
        main.runtime_dependencies.iter().any(|d| d.name == "helper"),
        "helper MUST be a runtime dependency of opt-in-fixture once \
         [package.metadata.pleme].features = [\"extra\"] opts it in — \
         got {:?}. If this fails, the --features pass-through in \
         generate_for_target regressed.",
        main.runtime_dependencies
            .iter()
            .map(|d| d.name.as_str())
            .collect::<Vec<_>>()
    );

    // And the activated dep's own crate entry must be present so the
    // Nix consumer can actually build it.
    assert!(
        spec.crates.values().any(|c| c.name == "helper"),
        "helper's own CrateSpec must be present in the emitted spec \
         once activated"
    );
}

/// Sanity anchor for the `sqlx-sqlite`-style precedent this fix is
/// deliberately NOT allowed to regress: an optional dep that no
/// feature (default OR opted-in) activates must still be pruned. Reuses
/// the fixture's `helper` dep but never asks for `extra` — equivalent
/// to `without_opt_in_the_gated_optional_dep_is_pruned` but phrased
/// against the documented regression class explicitly, so a future
/// reader searching for "sqlx-sqlite" finds the guard.
#[test]
fn unactivated_optional_dep_stays_pruned_like_sqlx_sqlite() {
    let dir = write_fixture(false);
    let spec = build_spec::generate_for_target(dir.path(), host_triple())
        .expect("spec generation (offline, path-only fixture)");
    let helper_present = spec.crates.values().any(|c| c.name == "helper");
    assert!(
        !helper_present,
        "helper's CrateSpec must be ABSENT when nothing activates it — \
         a blanket CargoOpt::AllFeatures fix would have broken this \
         exact invariant fleet-wide (the sqlx-sqlite precedent \
         documented in build_spec.rs)"
    );
}
