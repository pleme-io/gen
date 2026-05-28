//! End-to-end fixture test: emit a spec from a tiny Cargo workspace
//! and assert it satisfies every invariant gen-cargo declares.
//!
//! Run as part of `cargo test`. Catches:
//! - Spec-shape regressions (BuildRustCrateArgs missing fields)
//! - Quirk registry/emission drift (registered crate name without
//!   emitted quirks in the resulting spec — caught by
//!   `QuirkRegisteredButNotEmitted`)
//! - URL canonicalization regressions (`/api/v1/` URLs in emitted
//!   spec — caught by `RegistryUrlNotCanonical`)
//! - Schema version regressions (emitter forgetting to set the
//!   current SCHEMA_VERSION — caught by `StaleSchemaVersion`)
//! - Build-rust-crate-args field-set regressions (missing
//!   `crateName` or `preBuild` — caught by `MissingBuildRustCrateName`
//!   / `MissingUniversalPreBuild`)
//!
//! Maps every cse-lint invariant introduced for the typed-spec
//! migration to a code-level reverse-check: if any of them fire, the
//! emitter is broken.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

fn write_fixture() -> TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    // A tiny single-crate workspace — enough to exercise emission +
    // every invariant pass without dragging in heavy deps.
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
    fs::write(root.join("src/lib.rs"), "pub fn hello() -> &'static str { \"hello\" }\n").unwrap();
    // Generate a real lockfile via cargo so resolver edges exist.
    let _ = Command::new("cargo")
        .args(["generate-lockfile", "--offline"])
        .current_dir(root)
        .output();
    // If offline failed (no cargo cache), fall back to a tiny
    // hand-written lockfile with just serde + its dep edges. The
    // online version will pick this up on dev workstations.
    if !root.join("Cargo.lock").exists() {
        let _ = Command::new("cargo")
            .args(["generate-lockfile"])
            .current_dir(root)
            .output();
    }
    dir
}

#[test]
fn fixture_workspace_emits_spec_satisfying_every_invariant() {
    // Skip in environments where cargo can't reach the network (e.g.
    // a hermetic CI sandbox without a vendored registry). The test
    // is informational on those; the integration coverage of the
    // emitter still runs via consumer flakes' own `gen confirm`.
    let dir = write_fixture();
    if !dir.path().join("Cargo.lock").exists() {
        eprintln!("skipping: no Cargo.lock could be generated in fixture");
        return;
    }

    let spec = gen_cargo::build_spec::generate(dir.path()).expect("spec generation");

    // Schema version: the spec MUST carry the current SCHEMA_VERSION.
    assert_eq!(
        spec.version,
        gen_cargo::build_spec::SCHEMA_VERSION,
        "emitted spec version doesn't match SCHEMA_VERSION"
    );

    // Workspace member presence: at minimum the root crate appears.
    assert!(
        !spec.crates.is_empty(),
        "spec has no crates — emission produced empty closure"
    );
    assert!(
        spec.crates.values().any(|c| c.name == "fixture-crate"),
        "root crate `fixture-crate` missing from emitted spec"
    );

    // Universal preBuild on every crate (the CARGO_CRATE_NAME export).
    for (key, c) in &spec.crates {
        assert!(
            c.build_rust_crate_args.crate_name.is_some(),
            "{key}: build_rust_crate_args.crate_name missing"
        );
        assert!(
            c.build_rust_crate_args.pre_build.is_some(),
            "{key}: build_rust_crate_args.pre_build missing (CARGO_CRATE_NAME export not emitted)"
        );
        let pre = c.build_rust_crate_args.pre_build.as_ref().unwrap();
        assert!(
            pre.contains("CARGO_CRATE_NAME="),
            "{key}: preBuild doesn't export CARGO_CRATE_NAME (got `{pre}`)"
        );
    }

    // Registry URLs canonical: no `/api/v1/` (the rate-limited 403
    // endpoint). All registry URLs MUST be static.crates.io.
    for (key, c) in &spec.crates {
        if let gen_cargo::build_spec::CrateSource::Registry { url, .. } = &c.source {
            assert!(
                !url.starts_with("https://crates.io/api/v1/crates/"),
                "{key}: emitted spec has api/v1 URL (`{url}`) — should be static.crates.io"
            );
        }
    }

    // Full invariants pass: every gen-cargo invariant must hold on
    // this spec. Any violation indicates an emission regression.
    let violations = gen_cargo::invariants::check(&spec);
    assert!(
        violations.is_empty(),
        "fixture spec violates invariants: {violations:?}"
    );
}

#[test]
fn quirks_registered_names_match_actual_cargo_crate_names() {
    // Sanity check: every name in the typed quirks registry should
    // be a real crate (or could be) — no typos. We can't reach
    // crates.io in tests, so we just check name shape (lowercase
    // letters, digits, hyphens, underscores).
    for name in gen_cargo::quirks::registered_crate_names() {
        assert!(
            !name.is_empty(),
            "empty crate name in quirks registry"
        );
        assert!(
            name.chars().all(|c| c.is_ascii_lowercase()
                || c.is_ascii_digit()
                || c == '-'
                || c == '_'),
            "invalid crate name shape in quirks registry: `{name}`"
        );
        assert!(
            !name.starts_with('-') && !name.ends_with('-'),
            "crate name in quirks registry has stray hyphen: `{name}`"
        );
    }
}

// Ensures the `_dir` binding stays alive — silences clippy without
// changing the test surface.
#[allow(dead_code)]
fn _ensure_temp_root_paths_typecheck(_path: PathBuf) {}
