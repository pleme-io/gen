//! `directive` — the Rust half of the ONE `go` directive predicate.
//!
//! # The contract
//!
//! This is a PURE predicate: no filesystem, no process, no network. It is
//! therefore correctly exempt from the `Environment` seam — a mockable seam
//! exists to make side effects observable, and there are none here. Adding one
//! would be ceremony.
//!
//! # The shared table is the point
//!
//! `substrate/lib/build/go/directive-vectors.json` is the ONE committed vector
//! table. `tests/directive_vectors.rs` reads that exact file, as does
//! substrate's `tests/directive-test.nix`. A single byte edit there must turn
//! BOTH suites red. If only one goes red, a second copy of the ordering rule
//! exists — which is precisely how `Go.gen.lock`'s producer and consumer came
//! to disagree on 12 of 13 fields.
//!
//! # The measured fact, stated precisely
//!
//! cmd/go orders a bare `1.N` STRICTLY BELOW `1.N.0`. But "bare-minor is
//! unsatisfiable" is too strong, and the difference decides the remediation.
//! Reproduced on go1.25.10 with a two-module local `replace`:
//!
//! ```text
//! GOFLAGS=-mod=mod       BUILDS, and go SILENTLY REWRITES go.mod 1.25 -> 1.25.0
//! GOFLAGS=-mod=readonly  go: updates to go.mod needed, disabled by -mod=readonly
//! ```
//!
//! Real, and what every hermetic Nix build hits — not a universal failure.

/// The closed verdict set. Adding an arm means adding a vector row for it;
/// `directive_vectors.rs` asserts every arm has one, because an arm with no
/// vector is a branch nothing exercises.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Satisfiable,
    BareMinor,
    AboveFleetToolchain,
    NoDirective,
    Unparseable,
}

impl Verdict {
    /// The wire name, identical to the `verdict` field in the shared table and
    /// to substrate's Nix strings. One spelling, three languages.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Satisfiable => "satisfiable",
            Self::BareMinor => "bare-minor",
            Self::AboveFleetToolchain => "above-fleet-toolchain",
            Self::NoDirective => "no-directive",
            Self::Unparseable => "unparseable",
        }
    }

    pub const ALL: [Verdict; 5] = [
        Verdict::Satisfiable,
        Verdict::BareMinor,
        Verdict::AboveFleetToolchain,
        Verdict::NoDirective,
        Verdict::Unparseable,
    ];
}

/// Compare two dotted version strings component-wise, matching Nix's
/// `builtins.compareVersions` for the shapes this predicate accepts.
fn cmp_version(a: &str, b: &str) -> std::cmp::Ordering {
    let mut ai = a.split('.');
    let mut bi = b.split('.');
    loop {
        match (ai.next(), bi.next()) {
            (None, None) => return std::cmp::Ordering::Equal,
            // FEWER components sorts BELOW more: this is the `1.N` < `1.N.0`
            // rule, and it is the whole reason this predicate exists.
            (None, Some(_)) => return std::cmp::Ordering::Less,
            (Some(_), None) => return std::cmp::Ordering::Greater,
            (Some(x), Some(y)) => {
                let (xn, yn) = (x.parse::<u64>().ok(), y.parse::<u64>().ok());
                match (xn, yn) {
                    (Some(xv), Some(yv)) if xv != yv => return xv.cmp(&yv),
                    (Some(_), Some(_)) => continue,
                    _ => match x.cmp(y) {
                        std::cmp::Ordering::Equal => continue,
                        other => return other,
                    },
                }
            }
        }
    }
}

/// Classify a module's `go` directive against the fleet toolchain.
pub fn classify(directive: &str, fleet_go: &str) -> Verdict {
    let d = directive.trim();
    if d.is_empty() {
        return Verdict::NoDirective;
    }
    let dots = d.chars().filter(|c| *c == '.').count();
    if dots != 1 && dots != 2 {
        return Verdict::Unparseable;
    }
    if d.split('.')
        .any(|p| p.is_empty() || p.parse::<u64>().is_err())
    {
        return Verdict::Unparseable;
    }
    if cmp_version(d, fleet_go) == std::cmp::Ordering::Greater {
        return Verdict::AboveFleetToolchain;
    }
    if dots == 1 {
        return Verdict::BareMinor;
    }
    Verdict::Satisfiable
}

/// Read the `go` directive out of a `go.mod`'s text. `None` when there is no
/// `go` line — a different fact from an empty directive, and the caller
/// decides what it means rather than this guessing.
pub fn directive_of(go_mod: &str) -> Option<&str> {
    go_mod
        .lines()
        .map(str::trim)
        .find_map(|l| l.strip_prefix("go "))
        .map(str::trim)
}
