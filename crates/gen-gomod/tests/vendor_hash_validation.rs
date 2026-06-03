//! STEP 2 correctness gate (the crux): the `vendorHash` the SHIPPED
//! prefetcher computes MUST equal the hash nixpkgs `buildGoModule`
//! expects.
//!
//! Marked `#[ignore]` because it runs `nix-build` (and, for the
//! divergence-documentation test, a real `go` subprocess). Run:
//!
//! ```sh
//! cargo test -p gen-gomod --test vendor_hash_validation -- --ignored
//! ```
//!
//! ## Result (documented)
//!
//! The in-Rust reconstruction (`GoModDownloadPrefetcher`) DIVERGES from
//! nixpkgs — proven by `in_rust_reconstruction_diverges_documented`.
//! Root cause: this nixpkgs' `goModules` defaults to
//! `proxyVendor = false`, so the hashed tree is the `go mod vendor`
//! extracted-source tree (`.go` files + `modules.txt`), not the
//! `cache/download` tree the reconstruction produced; the vendor tree
//! is also import-graph-sensitive.
//!
//! Therefore the SHIPPED mechanism is
//! `BuildGoModuleFakeHashPrefetcher` (correct by construction — the
//! producer of the hash is the consumer).
//! `default_vendor_hash_matches_nixpkgs` asserts the SHIPPED default
//! reproduces the ground truth exactly.

use std::path::Path;

use gen_gomod::vendor_prefetcher::{
    self, BuildGoModuleFakeHashPrefetcher, GoModDownloadPrefetcher, VendorPrefetcher,
};

const REAL_MODULE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/real-module");

/// The pinned nixpkgs source on this host (the channel the validation
/// was performed against). Overridable via `GEN_GOMOD_NIXPKGS`.
fn nixpkgs_ref() -> String {
    std::env::var("GEN_GOMOD_NIXPKGS")
        .unwrap_or_else(|_| "/nix/store/cckbw1nrhmgbagsdbv976fb5wm3ssqcb-source".to_string())
}

/// The ground truth obtained out-of-band by building the `goModules`
/// FOD with `vendorHash = lib.fakeHash` and reading the `got:` SRI.
/// (uuid v1.6.0, the `testdata/real-module` fixture.)
const GROUND_TRUTH_SRI: &str = "sha256-mGKxBRU5TPgdmiSx0DHEd0Ys8gsVD/YdBfbDdSVpC3U=";

/// THE STEP-2 GATE: the SHIPPED default prefetcher reproduces the
/// nixpkgs ground truth EXACTLY.
#[test]
#[ignore = "needs `nix-build`; run with --ignored"]
fn default_vendor_hash_matches_nixpkgs() {
    let root = Path::new(REAL_MODULE);
    let shipped = vendor_prefetcher::default_prefetcher();
    let got = shipped.prefetch(root).expect("default prefetcher succeeds");
    eprintln!("shipped: {}", got.sri);
    eprintln!("truth  : {GROUND_TRUTH_SRI}");
    assert_eq!(
        got.sri, GROUND_TRUTH_SRI,
        "SHIPPED default_prefetcher() must equal the buildGoModule ground truth"
    );
}

/// Same assertion via the explicit fakeHash prefetcher type (proves the
/// shipped mechanism IS the fakeHash one).
#[test]
#[ignore = "needs `nix-build`; run with --ignored"]
fn fakehash_prefetcher_matches_nixpkgs() {
    let root = Path::new(REAL_MODULE);
    let probe = BuildGoModuleFakeHashPrefetcher {
        nixpkgs_ref: nixpkgs_ref(),
    };
    let got = probe.prefetch(root).expect("fakeHash probe succeeds");
    assert_eq!(got.sri, GROUND_TRUTH_SRI);
}

/// DOCUMENTATION of the divergence: the in-Rust reconstruction does NOT
/// match nixpkgs (that is WHY the default is the fakeHash mechanism).
/// Asserting the inequality keeps the documented rationale honest — if a
/// future change ever makes them equal, this test fails and prompts a
/// review of whether the cheaper in-Rust path can become the default.
#[test]
#[ignore = "needs `go` (network) + `nix-build`; run with --ignored"]
fn in_rust_reconstruction_diverges_documented() {
    let root = Path::new(REAL_MODULE);
    let in_rust = GoModDownloadPrefetcher::new()
        .prefetch(root)
        .expect("go mod download + NAR-sha256 succeeds");
    eprintln!("in-rust: {}", in_rust.sri);
    eprintln!("truth  : {GROUND_TRUTH_SRI}");
    assert_ne!(
        in_rust.sri, GROUND_TRUTH_SRI,
        "in-Rust reconstruction unexpectedly matches nixpkgs — revisit default_prefetcher()"
    );
}
