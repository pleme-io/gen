//! Production-scale skeleton round-trip: emit a skeleton for
//! every registered dispatcher in the catalog, write it next to
//! a copy of substrate's `mk-typed-dispatcher.nix`, and verify
//! `nix-instantiate --eval --strict` accepts it without error.
//!
//! Proves the emitter produces VALID Nix syntax for every typed
//! Rust enum in scope. Adding a new ecosystem with a Quirk enum
//! that the emitter can't handle fails this test before substrate
//! sees the change.

use std::path::Path;
use std::process::Command;

use gen_platform::catalog;

// Link every adapter for inventory.
use gen_ansible as _;
use gen_bundler as _;
use gen_cargo as _;
use gen_gomod as _;
use gen_helm as _;
use gen_npm as _;
use gen_pip as _;
use gen_poetry as _;
use gen_swift as _;

fn nix_available() -> bool {
    Command::new("nix-instantiate")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn substrate_shared_dispatcher() -> std::path::PathBuf {
    // Tests run from the workspace root, so substrate sits at
    // ../substrate relative to the gen crate dir.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../substrate/lib/build/shared/mk-typed-dispatcher.nix")
}

#[test]
fn every_catalog_entry_emits_valid_nix_skeleton() {
    if !nix_available() {
        eprintln!("nix-instantiate not on PATH; skipping round-trip test");
        return;
    }
    let shared = substrate_shared_dispatcher();
    if !shared.exists() {
        eprintln!(
            "substrate not found at {} — likely running outside the pleme-io tree; skipping round-trip test",
            shared.display()
        );
        return;
    }

    let tmp = tempfile::tempdir().expect("create tmpdir");
    // Mirror the substrate layout: lib/build/<eco>/quirk-apply.nix
    // imports ../shared/mk-typed-dispatcher.nix.
    let lib_build = tmp.path().join("lib").join("build");
    std::fs::create_dir_all(lib_build.join("shared")).unwrap();
    std::fs::copy(
        &shared,
        lib_build.join("shared").join("mk-typed-dispatcher.nix"),
    )
    .unwrap();

    let entries = catalog::registered();
    assert!(!entries.is_empty(), "catalog must have entries");

    let mut failed = Vec::new();
    for entry in &entries {
        let eco_dir = lib_build.join(&entry.label.replace('.', "-"));
        std::fs::create_dir_all(&eco_dir).unwrap();
        let path = eco_dir.join("quirk-apply.nix");
        let skel = gen_platform::to_helpers_skeleton(entry);
        std::fs::write(&path, &skel).unwrap();

        // Evaluate the skeleton — applyQuirks [] {} should yield {}.
        let output = Command::new("nix-instantiate")
            .args([
                "--eval",
                "--strict",
                "--show-trace",
                "-E",
                &format!(
                    "(import {} {{ lib = (import <nixpkgs> {{}}).lib; }}).applyQuirks [] {{}}",
                    path.display()
                ),
            ])
            .output()
            .expect("run nix-instantiate");
        if !output.status.success() {
            failed.push((
                entry.label,
                String::from_utf8_lossy(&output.stderr).into_owned(),
            ));
        }
    }

    assert!(
        failed.is_empty(),
        "skeleton emission failed nix-instantiate for: {failed:?}"
    );
}
