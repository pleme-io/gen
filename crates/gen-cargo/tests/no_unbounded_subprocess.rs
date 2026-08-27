//! ★ THE INVARIANT: gen must not grow a new way to hang.
//!
//! ── THE CLASS ────────────────────────────────────────────────────────
//! `Command::output()` / `.status()` / `.wait()` block forever. A child
//! that stalls mid-syscall sits at 0% CPU indefinitely and gen never
//! returns — indistinguishable, from outside, from work in progress.
//!
//! Measured 2026-08-27: `gen build` in `pangea-operator` wedged **twice**,
//! 20 minutes and 9 minutes at 0.0% CPU, with six orphaned
//! `git index-pack` children stalled mid-transfer. Reproducible, not bad
//! luck. With the fix, the same workspace on the same network completed
//! in **7m35s**.
//!
//! ── WHY A SOURCE GATE RATHER THAN A TYPE ─────────────────────────────
//! The honest tier: this is CI-caught, not unrepresentable.
//! `std::process::Command` is upstream and its blocking methods are
//! inherent, so they cannot be removed or deprecated from here. Making
//! the class truly unrepresentable would mean banning `std::process`
//! entirely behind a wrapper crate — which is the destination, but it is
//! a bigger move than this repair, and claiming it now would be a
//! round-up.
//!
//! ── WHY THIS TEST LIVES IN `tests/` ──────────────────────────────────
//! A source-scanning gate placed INSIDE the corpus it scans matches its
//! own matcher line — the line that greps for `.output()` contains
//! `.output()`. That is not hypothetical: `pangea-operator`'s
//! `every_apply_context_receives_the_shared_pacer` shipped with exactly
//! that bug and passed by COINCIDENCE for months, because its own assert
//! message happened to satisfy its own window. Adding a sibling test
//! broke the coincidence and turned both red.
//!
//! Living in `tests/` makes the self-match structurally impossible rather
//! than merely avoided: this file is not in the scanned corpus at all.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Blocking waits. `Command::new` is deliberately NOT here: building a
/// command is harmless, and `crate::bounded` itself must construct one.
/// It is the unbounded WAIT that hangs.
const BLOCKING: &[&str] = &[".output()", ".status()", ".wait()", ".spawn()"];

/// Files that still contain an unbounded wait.
///
/// ★ THIS LIST MAY ONLY SHRINK. The test asserts EXACT equality, so:
///   - a new file with an unbounded wait fails immediately, and
///   - converting a file fails until it is removed from this list,
/// which keeps the ledger honest in both directions. An allowlist that
/// is merely a superset rots into a permanent exemption.
///
/// Converted so far: `git_prefetcher.rs` (the measured wedge — network),
/// `adapter.rs` (`cargo generate-lockfile` — network),
/// `gen-cli/flake_lint.rs` (`nix flake metadata` — network). The
/// remainder are LOCAL git/process calls: lower risk, since they do not
/// wait on a remote, but not zero — a wedged filesystem or a stale
/// `index.lock` hangs them just as completely.
const PENDING: &[&str] = &[
    "gen-cargo/src/build_spec.rs",
    "gen-cargo/src/fleet_commit.rs",
    "gen-cargo/src/fleet_migrate.rs",
    "gen-cargo/src/fleet_verify.rs",
    "gen-cargo/src/path_resolver.rs",
    "gen-cli/src/main.rs",
];

fn crates_root() -> PathBuf {
    // CARGO_MANIFEST_DIR is …/crates/gen-cargo
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/")
        .to_path_buf()
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

fn offenders() -> BTreeSet<String> {
    let root = crates_root();
    let mut files = Vec::new();
    for c in ["gen-cargo", "gen-cli"] {
        walk(&root.join(c).join("src"), &mut files);
    }
    let mut out = BTreeSet::new();
    for f in files {
        // `bounded` IS the sanctioned implementation; it must contain
        // exactly the calls everything else is forbidden.
        if f.file_name().is_some_and(|n| n == "bounded.rs") {
            continue;
        }
        let Ok(src) = std::fs::read_to_string(&f) else {
            continue;
        };
        if BLOCKING.iter().any(|b| src.contains(b)) {
            let rel = f
                .strip_prefix(&root)
                .unwrap_or(&f)
                .to_string_lossy()
                .replace('\\', "/");
            out.insert(rel);
        }
    }
    out
}

#[test]
fn no_new_unbounded_subprocess_call_sites() {
    let found = offenders();
    let expected: BTreeSet<String> = PENDING.iter().map(|s| (*s).to_string()).collect();

    let new: Vec<&String> = found.difference(&expected).collect();
    assert!(
        new.is_empty(),
        "NEW unbounded subprocess wait(s) in {new:?}.\n\
         A raw .output()/.status()/.wait() blocks FOREVER — a stalled child \
         sits at 0% CPU and gen never returns, which is exactly the wedge \
         measured in pangea-operator on 2026-08-27.\n\
         Use `gen_cargo::bounded::Bounded::new(<deadline>)`; add \
         `.attempts(n).retry_failures(true)` for anything touching the network, \
         and pass `git_stall_guard()` to git so it aborts a dead transfer itself."
    );

    let fixed: Vec<&String> = expected.difference(&found).collect();
    assert!(
        fixed.is_empty(),
        "{fixed:?} no longer contains an unbounded wait — remove it from \
         PENDING. An allowlist that is allowed to be a superset stops being \
         a ledger and becomes a permanent exemption."
    );
}

/// The sanctioned module must actually be the thing it claims to be: if
/// `bounded.rs` stopped running subprocesses, every conversion above
/// would be pointing at nothing and the gate would still pass.
#[test]
fn the_sanctioned_module_is_not_vacuous() {
    let b = crates_root().join("gen-cargo/src/bounded.rs");
    let src = std::fs::read_to_string(&b).expect("bounded.rs must exist");
    assert!(
        src.contains("Command::new") || src.contains("cmd.spawn()") || src.contains(".spawn()"),
        "bounded.rs no longer spawns anything — the gate would be guarding \
         an empty rule"
    );
    assert!(
        src.contains("kill"),
        "bounded.rs must KILL at the deadline; a timeout that only reports \
         leaves the stalled child running"
    );
}
