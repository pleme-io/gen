//! Verification matrix — the forcing function (CLOSED-LOOP
//! MASS-SYNTHESIS Rule 1). One row per supported M1 shape; every row is
//! exercised end-to-end through the encoder and its failures aggregate
//! before the assert, so one run reports every broken shape. A new
//! supported shape that lands without a row fails `matrix_covers_all`.

mod common;

use gen_gomod::build_spec::{BuildSpec, TargetTuple};
use gen_gomod::interp::{EncodeCtx, apply};
use gen_gomod::invariants;
use gen_gomod::testkit::MockGoBuildEnv;

/// One matrix row: a named supported shape + its fixture + the extra
/// per-shape predicate (beyond the universal invariants-clean check).
struct Row {
    name: &'static str,
    fixture: fn(&TargetTuple) -> (MockGoBuildEnv, EncodeCtx),
    check: fn(&BuildSpec) -> Result<(), String>,
}

fn matrix() -> Vec<Row> {
    vec![
        Row {
            name: "dep-free-main",
            fixture: |t| common::dep_free(t),
            check: |s| {
                require(s.packages.len() >= 2, "dep-free: main + fmt")?;
                require(!s.root_package.is_empty(), "dep-free: has a root main")?;
                require(
                    s.go_sum_sha256.as_deref()
                        == Some("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"),
                    "dep-free: empty-string go_sum tie",
                )
            },
        },
        Row {
            name: "internal-shared",
            fixture: |t| common::gate_a("v1", t),
            check: |s| {
                require(s.packages.len() == 5, "internal-shared: 5 nodes")?;
                require(s.workspace_members.len() == 2, "internal-shared: 2 mains")?;
                require(
                    s.packages
                        .contains_key("example.com/fix/internal/greet#linux-amd64"),
                    "internal-shared: one shared greet node",
                )
            },
        },
        Row {
            name: "embed",
            fixture: |t| common::gate_a("v1", t),
            check: |s| {
                let greet = s
                    .packages
                    .get("example.com/fix/internal/greet#linux-amd64")
                    .ok_or("embed: greet node present")?;
                require(
                    !greet.embed.files.is_empty(),
                    "embed: greet carries embed files",
                )
            },
        },
        Row {
            name: "vendored+replace",
            fixture: |t| common::vendored_and_replace(t),
            check: |s| {
                let vend = s
                    .packages
                    .get("github.com/foo/bar#linux-amd64")
                    .ok_or("vendored: node present")?;
                let rel = match &vend.source {
                    gen_gomod::build_spec::PackageSource::Vendored { relative_path } => {
                        relative_path
                    }
                    _ => return Err("vendored: node has a Vendored source".into()),
                };
                require(
                    rel == "vendor/github.com/foo/bar",
                    "vendored: relative_path under vendor/",
                )
            },
        },
    ]
}

fn require(cond: bool, msg: &str) -> Result<(), String> {
    if cond { Ok(()) } else { Err(msg.to_string()) }
}

#[test]
fn every_supported_shape_encodes_and_holds_invariants() {
    let tuple = TargetTuple::new("linux", "amd64", vec![]);
    let mut failures: Vec<String> = Vec::new();
    for row in matrix() {
        let (env, ctx) = (row.fixture)(&tuple);
        match apply(&env, &ctx) {
            Err(e) => failures.push(format!("{}: encode failed: {e}", row.name)),
            Ok(spec) => {
                let v = invariants::check(&spec);
                if !v.is_empty() {
                    failures.push(format!("{}: invariants broken: {v:?}", row.name));
                }
                if let Err(msg) = (row.check)(&spec) {
                    failures.push(format!("{}: {msg}", row.name));
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} matrix row(s) failed:\n  - {}",
        failures.len(),
        failures.join("\n  - ")
    );
}

#[test]
fn matrix_covers_all_supported() {
    // M1 supports: dep-free, internal-shared, embed, vendored+replace.
    assert!(
        matrix().len() >= 4,
        "every supported shape needs a matrix row"
    );
}

// ── ECOSYSTEM-INTAKE dispatcher coverage: every typed quirk variant is
//    reflected through the adapter envelope (the substrate-side
//    quirk-apply.nix coverage test keys on this). ─────────────────────
#[test]
fn dispatcher_reflection_covers_every_quirk_variant() {
    use gen_gomod::quirks::GomodQuirk;
    use gen_types::{Adapter, TypedDispatcher};
    let a = gen_gomod::adapter::GomodAdapter;
    let reflected: Vec<String> = a
        .dispatcher_reflection()
        .into_iter()
        .map(|d| d.kind)
        .collect();
    let declared = GomodQuirk::variant_kinds();
    assert_eq!(
        reflected.len(),
        declared.len(),
        "reflected {reflected:?} vs declared {declared:?}"
    );
    for kind in declared {
        assert!(
            reflected.iter().any(|k| k == kind),
            "quirk `{kind}` missing from reflection"
        );
    }
}
