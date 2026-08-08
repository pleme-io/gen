//! `gen adopt-go` — the Go adoption census.
//!
//! # `--dry-run` is the only mode that exists
//!
//! There is no `--write`, no `--apply`, and no flag that could grow into one
//! without a code change. That is deliberate: this command's whole job is to
//! produce a NUMBER that a human then acts on. A census that could also mutate
//! is a migration tool wearing a census's name, and the failure mode — a
//! half-migrated fleet nobody chose — is exactly what a denominator is supposed
//! to prevent.
//!
//! The flag is still REQUIRED rather than implied, so the invocation states its
//! own harmlessness at the call site: `gen adopt-go --dry-run --all` reads as
//! read-only in a CI log without anyone knowing this file.
//!
//! # The fleet toolchain is an input, not a constant
//!
//! `--fleet-go` defaults to the value substrate's shared vector table records,
//! and the classification is `gen_gomod::directive` — the SAME predicate
//! substrate evaluates in Nix. A census that disagreed with the build gate
//! about what "eligible" means would be worse than no census.

use gen_gomod::adopt::{census, AdoptionRefusal, Census, FsAdoptEnv};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The fleet Go toolchain. Mirrors `fleetGo` in
/// `substrate/lib/build/go/directive-vectors.json`; `directive_vectors.rs`
/// asserts the two agree, so this cannot drift silently.
pub const DEFAULT_FLEET_GO: &str = "1.26.5";

/// A directory carrying this file, and everything beneath it, is excluded from
/// the census.
///
/// # Why this exists: the census counted its own instrument
///
/// Measured 2026-08-08, first run over the real workspace: the ONLY
/// `above-fleet-toolchain` offender in 433 module roots was
/// `gen-gomod/tests/fixtures/adopt/above-fleet` — a fixture written minutes
/// earlier *to be* an offender. The committed bare-minor fixture had the same
/// effect on the other arm, which is worse than cosmetic: the escalation
/// predicate is `bare-minor-directive == 0`, and a deliberately-broken input
/// living in the tree makes 0 **unreachable by construction**. A done-predicate
/// that can never fire is not a done-predicate.
///
/// # Why a marker file and not a path pattern
///
/// `**/tests/fixtures/**` is a hand-list of somebody else's layout convention;
/// it rots the first time a fixture lands somewhere else. A marker is declared
/// by the tree that wants excluding, so it travels with it.
///
/// The failure directions are deliberately asymmetric. A fixture that forgets
/// the marker is COUNTED — the census over-reports, someone investigates, and
/// finds a fixture. A real module that gained one would be silently dropped,
/// which is why the marker is explicit, rare, and greppable.
const IGNORE_MARKER: &str = ".gen-adopt-ignore";

/// Discover module roots: every directory containing a `go.mod`.
///
/// Skips `vendor/` (its `modules.txt` names dependencies, not our modules — a
/// vendored tree would inflate the denominator with other people's modules),
/// dot-directories, and any tree marked with [`IGNORE_MARKER`]. Discovery,
/// never a hand-list: a repo added tomorrow is counted the day it lands.
fn discover(root: &Path, out: &mut Vec<String>) {
    if root.join(IGNORE_MARKER).is_file() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    if root.join("go.mod").is_file() {
        out.push(root.display().to_string());
    }
    for e in entries.flatten() {
        let p = e.path();
        if !p.is_dir() {
            continue;
        }
        let name = e.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || name == "vendor" || name == "node_modules" {
            continue;
        }
        discover(&p, out);
    }
}

pub struct AdoptGoArgs {
    pub roots: Vec<PathBuf>,
    pub fleet_go: String,
    pub json: bool,
}

/// Run the census. Returns the exit code: non-zero when anything was refused,
/// so CI does not need to parse the output to notice.
pub fn run(args: &AdoptGoArgs) -> Result<i32, String> {
    let mut roots: Vec<String> = Vec::new();
    for r in &args.roots {
        discover(r, &mut roots);
    }
    roots.sort();
    roots.dedup();

    let c = census(&FsAdoptEnv, &roots, &args.fleet_go);

    // A census over ZERO roots is not a pass, and it must refuse BEFORE
    // printing: a report reading `bare-minor-directive == 0` above an error is
    // the exact line somebody greps for, and it would be a lie. A scan that
    // found nothing looks identical to a fleet with no problems, and telling
    // those apart is the whole point of printing a denominator.
    if c.scanned == 0 {
        return Err(
            "adopt-go: scanned 0 module roots. Nothing was measured, which is not the \
             same as nothing being wrong — check the --root paths."
                .into(),
        );
    }

    if args.json {
        print_json(&c, &args.fleet_go);
    } else {
        print_human(&c, &args.fleet_go);
    }

    Ok(i32::from(!c.refused.is_empty()))
}

fn by_reason(c: &Census) -> BTreeMap<&'static str, Vec<&str>> {
    let mut m: BTreeMap<&'static str, Vec<&str>> = BTreeMap::new();
    for (root, reason) in &c.refused {
        m.entry(reason.as_str()).or_default().push(root.as_str());
    }
    m
}

fn print_human(c: &Census, fleet_go: &str) {
    println!("gen adopt-go --dry-run   (fleet Go {fleet_go})");
    println!("  scanned  {}", c.scanned);
    println!("  eligible {}", c.eligible);
    println!("  refused  {}", c.refused.len());
    for (reason, roots) in by_reason(c) {
        println!("\n  {reason}  ({})", roots.len());
        for r in roots {
            println!("    {r}");
        }
        if reason == AdoptionRefusal::BareMinorDirective.as_str() {
            println!(
                "    ^ remediation is one line per module: `go 1.N` -> `go 1.N.0`.\n\
                 \x20     Builds under -mod=mod (go rewrites go.mod silently); FAILS under\n\
                 \x20     -mod=readonly / -mod=vendor, which is every hermetic Nix build.\n\
                 \x20     substrate's build-time trace escalates to a throw when this reaches 0."
            );
        }
    }
    println!("\n  bare-minor-directive == {}  <- the escalation predicate", c.bare_minor_directive());
}

fn print_json(c: &Census, fleet_go: &str) {
    let refused: Vec<serde_json::Value> = c
        .refused
        .iter()
        .map(|(root, reason)| serde_json::json!({ "root": root, "reason": reason.as_str() }))
        .collect();
    let v = serde_json::json!({
        "fleet_go": fleet_go,
        "scanned": c.scanned,
        "eligible": c.eligible,
        "refused": refused,
        "bare_minor_directive": c.bare_minor_directive(),
    });
    println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
}
