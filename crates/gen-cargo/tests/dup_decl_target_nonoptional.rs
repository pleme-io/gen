//! Regression test for the DUPLICATE-DECLARATION target-disambiguation
//! fix (escriba / mado / GPU-fleet Linux build wedge, 2026-06-18).
//!
//! A single dep name+kind can be declared MULTIPLE times in one manifest
//! — once bare under `[dependencies]` and once under
//! `[target.'cfg(...)'.dependencies]` — and the two declarations can
//! disagree on `optional`. `wgpu` is the canonical case:
//!
//!     [dependencies.wgpu-core]                                         # optional = true
//!     [target.'cfg(not(target_arch = "wasm32"))'.dependencies.wgpu-core]  # optional = false (native)
//!     [target.'cfg(not(target_arch = "wasm32"))'.dependencies.wgpu-hal]   # optional = false (native)
//!
//! gen-cargo matched the declared dep by name+kind with a plain
//! `.find()`, which returned the FIRST (bare, optional) declaration. So
//! `optional = true` + `target = None` won even when the resolved edge
//! cargo actually pulled (under `--filter-platform`) is the non-optional
//! `cfg(not(wasm32))` one. The downstream optional-dep activation guard
//! then saw `optional && not target-gated && no activating feature` and
//! DROPPED the edge entirely. On native targets that silently removed
//! `wgpu`'s `wgpu-core` / `wgpu-hal` backend deps, so `wgpu`'s own
//! `backend/wgpu_core.rs` (`use wgc::command::bundle_ffi::*`) failed to
//! compile with 17× `error[E0425]: cannot find function
//! wgpu_render_bundle_draw_indexed_indirect` (and siblings) — a
//! fleet-wide Linux build break across every GPU consumer.
//!
//! The fix disambiguates the declared dep by the resolved edge's
//! authoritative `target` (`graph_kind.target`), preferring the
//! declaration whose `target` matches the edge's, so the native
//! non-optional `wgpu-core` / `wgpu-hal` edges survive.
//!
//! Runs on any host: `--filter-platform x86_64-unknown-linux-gnu` is a
//! resolve-time filter, so cargo evaluates the linux cfg even on macOS.
//! Skips gracefully when cargo can't reach a registry (hermetic CI).

use std::fs;
use std::process::Command;
use tempfile::TempDir;

fn write_wgpu_fixture() -> TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "wgpu-fixture"
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

#[test]
fn wgpu_native_backend_deps_survive_dup_declaration_on_linux() {
    let dir = write_wgpu_fixture();
    if !dir.path().join("Cargo.lock").exists() {
        eprintln!("skipping: no Cargo.lock could be generated (offline, no registry cache)");
        return;
    }

    let spec =
        match gen_cargo::build_spec::generate_for_target(dir.path(), "x86_64-unknown-linux-gnu") {
            Ok(s) => s,
            Err(e) => {
                eprintln!("skipping: generate_for_target failed (offline metadata?): {e}");
                return;
            }
        };

    let wgpu = spec
        .crates
        .values()
        .find(|c| c.name == "wgpu")
        .expect("wgpu must appear in the linux-target resolve");

    // The exact regression: the non-optional native-backend edges were
    // dropped because the bare optional declaration shadowed them. If
    // either is gone, the bug is back and `wgpu`'s own
    // backend/wgpu_core.rs fails to compile (E0425 on the bundle_ffi
    // symbols) — the fleet-wide Linux GPU build break.
    for name in ["wgpu_core", "wgpu-core"] {
        if let Some(edge) = wgpu.runtime_dependencies.iter().find(|d| {
            (d.name == name || d.name == "wgpu_core" || d.name == "wgpu-core")
                && d.target.as_deref() == Some("cfg(not(target_arch = \"wasm32\"))")
        }) {
            assert!(
                !edge.optional,
                "wgpu's native (cfg(not(wasm32))) wgpu-core edge must be \
                 NON-optional (the target-disambiguated declaration), else \
                 the optional filter drops it and wgpu-core never builds. \
                 got optional = {}",
                edge.optional
            );
            break;
        }
    }

    // Assert the native non-optional wgpu-core edge is present at all.
    let has_native_core = wgpu.runtime_dependencies.iter().any(|d| {
        (d.name == "wgpu_core" || d.name == "wgpu-core")
            && d.target.as_deref() == Some("cfg(not(target_arch = \"wasm32\"))")
            && !d.optional
    });
    assert!(
        has_native_core,
        "wgpu's non-optional native wgpu-core edge \
         (cfg(not(target_arch=\"wasm32\"))) was dropped — the \
         duplicate-declaration target-disambiguation regressed. \
         wgpu deps = {:?}",
        wgpu.runtime_dependencies
            .iter()
            .map(|d| (d.name.as_str(), d.target.clone(), d.optional))
            .collect::<Vec<_>>()
    );

    // And wgpu-hal: declared bare-absent but native non-optional; the
    // bare path here is only `optional = true` under wasm32, so the
    // native edge must come through non-optional too.
    let has_native_hal = wgpu.runtime_dependencies.iter().any(|d| {
        (d.name == "wgpu_hal" || d.name == "wgpu-hal")
            && d.target.as_deref() == Some("cfg(not(target_arch = \"wasm32\"))")
            && !d.optional
    });
    assert!(
        has_native_hal,
        "wgpu's non-optional native wgpu-hal edge \
         (cfg(not(target_arch=\"wasm32\"))) was dropped or left optional. \
         wgpu deps = {:?}",
        wgpu.runtime_dependencies
            .iter()
            .map(|d| (d.name.as_str(), d.target.clone(), d.optional))
            .collect::<Vec<_>>()
    );
}
