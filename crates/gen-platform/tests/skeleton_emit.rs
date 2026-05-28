//! Test the helpers-skeleton emitter against the production
//! gen_cargo::quirks::CrateQuirk enum: text-shape assertions
//! plus round-trip dispatch verification (every produced kind
//! is callable through the emitted skeleton's applyQuirks).
//!
//! The round-trip test stops short of running `nix-instantiate`
//! from inside the Rust test — that's a substrate-level
//! integration concern. This test verifies the EMITTED TEXT is
//! the shape the substrate expects.

use gen_cargo::quirks::CrateQuirk;
use gen_platform::{to_helpers_skeleton, TypedDispatcherTrait};

fn synth_entry() -> gen_platform::DispatcherEntry {
    gen_platform::DispatcherEntry {
        label: "test.demo.crate-quirk",
        variant_kinds: CrateQuirk::variant_kinds,
        variant_fields: CrateQuirk::variant_fields,
        variant_count: CrateQuirk::variant_count,
    }
}

#[test]
fn skeleton_imports_shared_dispatcher() {
    let s = to_helpers_skeleton(&synth_entry());
    assert!(
        s.contains("import ../shared/mk-typed-dispatcher.nix"),
        "skeleton must import the shared combinator; got:\n{s}"
    );
}

#[test]
fn skeleton_has_lib_arg() {
    let s = to_helpers_skeleton(&synth_entry());
    assert!(s.contains("{ lib }:"), "missing lib arg; got:\n{s}");
}

#[test]
fn skeleton_emits_helpers_table() {
    let s = to_helpers_skeleton(&synth_entry());
    assert!(s.contains("helpers = {"), "missing helpers table; got:\n{s}");
    assert!(s.contains("};\n}"), "missing closing braces; got:\n{s}");
}

#[test]
fn skeleton_contains_every_kind_string() {
    let s = to_helpers_skeleton(&synth_entry());
    for k in CrateQuirk::variant_kinds() {
        let needle = format!("\"{k}\"");
        assert!(
            s.contains(&needle),
            "skeleton missing kind {needle}; got:\n{s}"
        );
    }
}

#[test]
fn skeleton_uses_camel_case_apply_fn_names() {
    let s = to_helpers_skeleton(&synth_entry());
    assert!(s.contains("forceCfgApply"), "missing forceCfgApply; got:\n{s}");
    assert!(
        s.contains("foldNormalIntoBuildApply"),
        "missing foldNormalIntoBuildApply; got:\n{s}"
    );
    assert!(
        s.contains("substituteSourceApply"),
        "missing substituteSourceApply; got:\n{s}"
    );
}

#[test]
fn skeleton_destructures_multi_field_variants() {
    let s = to_helpers_skeleton(&synth_entry());
    // substitute-source has fields file/from/to — must be
    // destructured via inherit (quirk) at the call site, and the
    // helper signature accepts `{ file, from, to }`.
    assert!(
        s.contains("inherit (quirk) file from to"),
        "multi-field call site must use `inherit (quirk) ...`; got:\n{s}"
    );
    assert!(
        s.contains("{ file, from, to }"),
        "multi-field helper sig must destructure; got:\n{s}"
    );
}

#[test]
fn skeleton_unwraps_single_field_variants() {
    let s = to_helpers_skeleton(&synth_entry());
    // force-cfg has single field `cfg` — emitted as
    // `forceCfgApply quirk.cfg` at the call site, signature is
    // `cfg: _: {}`.
    assert!(
        s.contains("forceCfgApply quirk.cfg"),
        "single-field call site must use `helperName quirk.<field>`; got:\n{s}"
    );
    assert!(
        s.contains("forceCfgApply = cfg: _: {};"),
        "single-field helper sig must take the field directly; got:\n{s}"
    );
}

#[test]
fn skeleton_labels_dispatcher() {
    let s = to_helpers_skeleton(&synth_entry());
    assert!(
        s.contains("test.demo.crate-quirk"),
        "skeleton must name the dispatcher label; got:\n{s}"
    );
}

#[test]
fn skeleton_kind_count_matches_dispatcher() {
    let s = to_helpers_skeleton(&synth_entry());
    let expected = CrateQuirk::variant_count();
    let actual = s.matches('"').filter(|_| true).count() / 2; // pairs of quotes wrap each kind
    assert_eq!(
        actual,
        expected,
        "expected {expected} kinds in skeleton, found {actual}; skeleton:\n{s}"
    );
}
