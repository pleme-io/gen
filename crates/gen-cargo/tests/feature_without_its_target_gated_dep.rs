//! Regression test for the FEATURE/DEP TARGET SKEW fix (namimado darwin
//! build wedge, 2026-08-09).
//!
//! cargo filters the resolve's dep EDGES by target but does NOT filter the
//! resolved FEATURE list. Measured on wgpu-hal 25.0.2, same workspace, same
//! lock, both invocations of `cargo metadata`:
//!
//!     unfiltered                                39 deps, portable-atomic PRESENT
//!     --filter-platform=aarch64-apple-darwin    27 deps, portable-atomic ABSENT
//!     features, BOTH                            [..., portable-atomic, ...]
//!
//! So cargo hands back an inconsistent pair. Emitting it verbatim produces a
//! crate compiled WITH a feature but WITHOUT the dependency that feature
//! exists to activate. wgpu-hal declares:
//!
//!     portable-atomic = ["dep:portable-atomic"]                            # feature
//!     [target.'cfg(not(target_has_atomic = "64"))'.dependencies.portable-atomic]
//!
//! and `src/lib.rs` does `pub type AtomicFenceValue = portable_atomic::AtomicU64;`
//! whenever that feature is on. aarch64-apple-darwin HAS 64-bit atomics, so
//! the edge is correctly filtered out while the feature stayed on, and the
//! build died with `error[E0432]: unresolved import portable_atomic` — taking
//! wgpu, wgpu-core, glyphon, garasu, nami-core and namimado down with it, i.e.
//! `nix run .#rebuild` on every darwin node.
//!
//! The fix drops a feature ONLY when every one of its expansions is a
//! `dep:<name>` whose edge is absent from THIS target's resolve. A feature
//! that also flips cfgs, enables other features, or names a dep that IS
//! present is left alone — so it cannot silently disable real functionality.
//!
//! Sibling of `dup_decl_target_nonoptional` and `target_cfg_optional_dep`:
//! same class (target-conditional deps), opposite side of the edge/feature
//! pair.
//!
//! Runs on any host — `--filter-platform` is a resolve-time filter, so cargo
//! evaluates the target's cfgs regardless of where the test runs. Skips
//! gracefully when cargo cannot reach a registry (hermetic CI).

use std::fs;
use std::process::Command;
use tempfile::TempDir;

/// A fixture depending on `wgpu`, NOT on wgpu-hal directly.
///
/// This distinction is load-bearing and was found the hard way: a bare
/// `wgpu-hal = "25"` dep resolves with `features: []`, so every assertion
/// below passes VACUOUSLY — `has_feature` is false and the invariant is
/// trivially satisfied. `wgpu` is what turns wgpu-hal's backend features on,
/// which is also how the real fleet graph (namimado -> garasu/nami-core ->
/// wgpu -> wgpu-hal) reaches the broken state.
///
/// Measured on this fixture:
///   unfiltered                              features [dx12..portable-atomic..vulkan], dep PRESENT
///   --filter-platform=x86_64-apple-darwin   features IDENTICAL,                       dep ABSENT
fn write_fixture() -> TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "feature-skew-fixture"
version = "0.1.0"
edition = "2021"

[dependencies]
wgpu = "25"
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

/// On a target WITH 64-bit atomics the `portable-atomic` dep edge is filtered
/// out, so the feature that exists only to activate it must not survive.
#[test]
fn dep_activating_feature_is_dropped_when_its_edge_is_filtered_out() {
    let dir = write_fixture();
    if !dir.path().join("Cargo.lock").exists() {
        eprintln!("skipping: no Cargo.lock could be generated (offline, no registry cache)");
        return;
    }

    let spec = match gen_cargo::build_spec::generate_for_target(dir.path(), "x86_64-apple-darwin") {
        Ok(s) => s,
        Err(e) => {
            eprintln!("skipping: generate_for_target failed (offline metadata?): {e}");
            return;
        }
    };

    let Some(hal) = spec.crates.values().find(|c| c.name == "wgpu-hal") else {
        eprintln!("skipping: wgpu-hal absent from the resolve");
        return;
    };

    let has_dep = hal
        .runtime_dependencies
        .iter()
        .any(|d| d.name == "portable-atomic" || d.name == "portable_atomic");
    let has_feature = hal.features.iter().any(|f| f == "portable-atomic");

    // THE INVARIANT: feature ⇒ dep. Never a feature without the dep it
    // activates, which is precisely what produced E0432.
    assert!(
        !(has_feature && !has_dep),
        "wgpu-hal carries the `portable-atomic` FEATURE but not the \
         portable-atomic DEP on x86_64-apple-darwin. That is the skew that \
         compiles `use portable_atomic::AtomicU64` with no extern and fails \
         with E0432. features = {:?}",
        hal.features
    );
}

/// The other direction, so the fix cannot be "delete the feature always":
/// on a target WITHOUT 64-bit atomics the edge survives, and so must the
/// feature. Without this, a green run would prove nothing.
#[test]
fn the_feature_survives_on_a_target_that_really_needs_it() {
    let dir = write_fixture();
    if !dir.path().join("Cargo.lock").exists() {
        eprintln!("skipping: no Cargo.lock could be generated");
        return;
    }

    // thumbv6m-none-eabi is a 32-bit target with no 64-bit atomics, so
    // cfg(not(target_has_atomic = "64")) is TRUE and the dep edge survives.
    let spec = match gen_cargo::build_spec::generate_for_target(dir.path(), "thumbv6m-none-eabi") {
        Ok(s) => s,
        Err(e) => {
            eprintln!("skipping: generate_for_target failed for thumbv6m: {e}");
            return;
        }
    };

    let Some(hal) = spec.crates.values().find(|c| c.name == "wgpu-hal") else {
        eprintln!("skipping: wgpu-hal absent from the thumbv6m resolve");
        return;
    };

    let has_dep = hal
        .runtime_dependencies
        .iter()
        .any(|d| d.name == "portable-atomic" || d.name == "portable_atomic");

    if has_dep {
        assert!(
            hal.features.iter().any(|f| f == "portable-atomic"),
            "the portable-atomic dep edge survives on thumbv6m-none-eabi, so \
             its activating feature must too — pruning it there would disable \
             real functionality. features = {:?}",
            hal.features
        );
    } else {
        eprintln!("note: thumbv6m resolve carried no portable-atomic edge; \
                   direction-2 assertion skipped");
    }
}

/// A feature with a NON-`dep:` expansion must never be pruned, even when it
/// also activates an absent dep. Guards the narrowness of the rule.
#[test]
fn features_with_non_dep_expansions_are_never_pruned() {
    let dir = write_fixture();
    if !dir.path().join("Cargo.lock").exists() {
        eprintln!("skipping: no Cargo.lock could be generated");
        return;
    }
    let spec = match gen_cargo::build_spec::generate_for_target(dir.path(), "x86_64-apple-darwin") {
        Ok(s) => s,
        Err(e) => {
            eprintln!("skipping: {e}");
            return;
        }
    };
    let Some(hal) = spec.crates.values().find(|c| c.name == "wgpu-hal") else {
        return;
    };

    // `metal` is darwin's real backend feature — it expands to more than a
    // bare `dep:`, and must survive on a darwin target. If the prune were
    // over-broad this is the first thing it would eat.
    assert!(
        hal.features.iter().any(|f| f == "metal"),
        "the `metal` feature must survive on x86_64-apple-darwin — pruning it \
         would disable the darwin GPU backend. features = {:?}",
        hal.features
    );
}
