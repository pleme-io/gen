//! GEN TYPED-SPEC CONTRACT — I4: the build spec carries no host path.
//!
//! `Cargo.build-spec.json` is a content-addressed BUILD INPUT. Every byte in
//! it must be a function of the source tree and nothing else. Until v11 it
//! carried `workspace.root`, an absolute host path, so the same source
//! emitted three different specs across the fleet — observed live as
//! `/private/tmp/conv-<repo>`, `/Users/<user-a>/…` and `/Users/<user-b>/…`.
//! Two correct machines disagreed about identical source, and every genuine
//! dependency change arrived buried in that noise.
//!
//! Per the contract's shape (an encoder test per conditional emission rule),
//! these are the encoder-side assertions. They are deliberately split into
//! the two directions that matter to the fleet, because only one of them is
//! about gen and the other is about the 13+ repos with a committed spec:
//!
//!   * FORWARD  — what gen emits today carries no `root`.
//!   * BACKWARD — what those repos committed yesterday still parses.
//!
//! The second is the one that would break every Rust build at once if it
//! were wrong, so it is tested against the real committed v10 fixture rather
//! than a hand-built value.

use gen_cargo::build_spec::{BuildSpec, WorkspaceSpec};
use serde_json::Value;

/// The genuine pre-v11 artifact, committed. Carries `workspace.root` — that
/// is precisely why it is the right witness here and must not be "fixed".
const V10_FIXTURE: &str = include_str!("../src/testdata/v10-build-spec.json");

/// BACKWARD: a committed pre-v11 spec still parses.
///
/// This is the fleet-compat claim in its load-bearing form. `root` becomes a
/// plain unknown key; serde ignores it because no `deny_unknown_fields`
/// exists in this crate. If this ever fails, every consumer repo holding a
/// pre-v11 `Cargo.build-spec.json` fails to parse at once.
#[test]
fn a_committed_pre_v11_spec_carrying_root_still_parses() {
    let raw: Value = serde_json::from_str(V10_FIXTURE).expect("fixture is valid JSON");
    assert!(
        raw["workspace"]["root"].is_string(),
        "the fixture must actually carry workspace.root, or it witnesses nothing",
    );

    let spec: BuildSpec =
        serde_json::from_str(V10_FIXTURE).expect("a pre-v11 spec carrying `root` must still parse");
    assert!(
        !spec.crates.is_empty(),
        "the fixture must decode to a populated spec, not an empty shell",
    );
}

/// FORWARD: re-emitting that same spec drops the host path.
///
/// This is the migration every consumer takes — ingest the old spec, emit the
/// new one — so asserting on a *round-trip of the real fixture* proves more
/// than serializing a hand-built value would: the machine-specific byte does
/// not survive the trip even when the input supplies one.
#[test]
fn re_emitting_a_pre_v11_spec_drops_the_host_path() {
    let spec: BuildSpec = serde_json::from_str(V10_FIXTURE).expect("fixture parses");
    let out: Value = serde_json::to_value(&spec).expect("spec serializes");

    assert!(
        out["workspace"].is_object(),
        "workspace must still be an object — this drops a field, not the struct",
    );
    assert!(
        out["workspace"].get("root").is_none(),
        "a re-emitted spec must not carry workspace.root; got {}",
        out["workspace"],
    );
}

/// The emitter has no path back to a `root` key at all.
///
/// Serializing the type directly is the tightest statement available in
/// Rust: the field is absent from the struct, so no value of `WorkspaceSpec`
/// can produce the key. Removing the field rather than blanking it is what
/// buys this — a relativized `root` would still be a live field one careless
/// edit could refill with `root.display()`.
#[test]
fn the_workspace_type_cannot_emit_a_root_key() {
    let out = serde_json::to_value(WorkspaceSpec {
        members: Vec::new(),
    })
    .expect("serializes");

    assert!(
        out.get("root").is_none(),
        "WorkspaceSpec must have no `root` key; got {out}",
    );
}

/// The CLASS, not the instance: no absolute host path anywhere in a spec.
///
/// `root` was one field. The defect is "a machine-specific byte reached a
/// content-addressed artifact", and a test naming only `root` would go green
/// the day someone adds a second such field. This walks every string in a
/// re-emitted real spec and rejects the host-path shapes the fleet sweep
/// actually observed.
///
/// Scoped deliberately to the host-path prefixes rather than "any string
/// starting with `/`": a spec legitimately carries `/nix/store/…` and
/// registry URLs, and a test that fails on those would be noise, not a gate.
#[test]
fn no_host_path_survives_anywhere_in_a_re_emitted_spec() {
    let spec: BuildSpec = serde_json::from_str(V10_FIXTURE).expect("fixture parses");
    let out = serde_json::to_value(&spec).expect("spec serializes");

    // The shapes the fleet sweep actually observed across 8 repos.
    const HOST_PREFIXES: &[&str] = &["/Users/", "/home/", "/private/tmp/", "/tmp/conv-"];

    let mut offenders: Vec<String> = Vec::new();
    walk(&out, &mut |s| {
        if let Some(p) = HOST_PREFIXES.iter().find(|p| s.starts_with(**p)) {
            offenders.push(format!("{p} … in {s:?}"));
        }
    });

    assert!(
        offenders.is_empty(),
        "a re-emitted spec must carry no absolute host path; found {}:\n  - {}",
        offenders.len(),
        offenders.join("\n  - "),
    );
}

/// Visit every string in a JSON value, including object keys' values and
/// nested arrays. Aggregates before asserting so one run reports every
/// offender rather than only the first.
fn walk(v: &Value, f: &mut impl FnMut(&str)) {
    match v {
        Value::String(s) => f(s),
        Value::Array(items) => items.iter().for_each(|i| walk(i, f)),
        Value::Object(map) => map.values().for_each(|i| walk(i, f)),
        _ => {}
    }
}
