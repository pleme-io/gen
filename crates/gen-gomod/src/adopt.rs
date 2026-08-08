//! `adopt` — the Go adoption census. Measures; adopts nothing.
//!
//! # Why a census before a migration
//!
//! "Bring all our Go projects onto gen" is 97 repos in the ask and a much
//! smaller number in reality. This prints the denominator with refusals by
//! NAMED REASON, so the wave is chosen from a count rather than from a
//! name-glob — the plan's first cut picked `*-go`/`go-*`/`borealis-*` and
//! roughly half of that set turned out to be bare-minor.
//!
//! # The refusal is typed, not a bool
//!
//! `AdoptionVerdict` has exactly two arms and `Refused` CARRIES its reason, so
//! *refused with no reason* has no representation. A census whose refusals
//! degrade to a count is one that cannot tell you what to fix.

use crate::directive::{classify, Verdict};

/// Why a module cannot be adopted yet. Closed: a new refusal is a new arm and
/// a match arm everywhere, which is the point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdoptionRefusal {
    /// `go 1.N` — fails under -mod=readonly/vendor, which is every hermetic build.
    BareMinorDirective,
    /// Requires a newer Go than the fleet toolchain; cmd/go refuses outright.
    AboveFleetToolchain,
    /// No `go` line at all.
    NoDirective,
    /// A `go` directive this predicate cannot parse.
    UnparseableDirective,
    /// No `go.mod` at the module root.
    NoRootGoMod,
}

impl AdoptionRefusal {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BareMinorDirective => "bare-minor-directive",
            Self::AboveFleetToolchain => "above-fleet-toolchain",
            Self::NoDirective => "no-directive",
            Self::UnparseableDirective => "unparseable-directive",
            Self::NoRootGoMod => "no-root-go-mod",
        }
    }
}

/// Two arms. `Refused` carries its reason, so a reasonless refusal cannot be
/// constructed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdoptionVerdict {
    Eligible,
    Refused(AdoptionRefusal),
}

/// The seam. `adopt-go` reads go.mod files and nothing else, so this is the
/// whole observable surface — and it makes the census testable with no disk.
pub trait AdoptEnv {
    /// The `go.mod` text for a module root, or `None` when absent.
    fn read_go_mod(&self, module_root: &str) -> Option<String>;
}

/// Filesystem implementation.
pub struct FsAdoptEnv;

impl AdoptEnv for FsAdoptEnv {
    fn read_go_mod(&self, module_root: &str) -> Option<String> {
        std::fs::read_to_string(std::path::Path::new(module_root).join("go.mod")).ok()
    }
}

/// Classify one module root.
pub fn assess<E: AdoptEnv>(env: &E, module_root: &str, fleet_go: &str) -> AdoptionVerdict {
    let Some(text) = env.read_go_mod(module_root) else {
        return AdoptionVerdict::Refused(AdoptionRefusal::NoRootGoMod);
    };
    let directive = crate::directive::directive_of(&text).unwrap_or("");
    match classify(directive, fleet_go) {
        Verdict::Satisfiable => AdoptionVerdict::Eligible,
        Verdict::BareMinor => AdoptionVerdict::Refused(AdoptionRefusal::BareMinorDirective),
        Verdict::AboveFleetToolchain => {
            AdoptionVerdict::Refused(AdoptionRefusal::AboveFleetToolchain)
        }
        Verdict::NoDirective => AdoptionVerdict::Refused(AdoptionRefusal::NoDirective),
        Verdict::Unparseable => AdoptionVerdict::Refused(AdoptionRefusal::UnparseableDirective),
    }
}

/// The census: the denominator, and refusals by named reason.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Census {
    pub scanned: usize,
    pub eligible: usize,
    pub refused: Vec<(String, AdoptionRefusal)>,
}

impl Census {
    /// The escalation predicate for substrate's bare-minor trace: when this
    /// reaches 0, the trace may become a throw. A COUNT, not a judgement call.
    pub fn bare_minor_directive(&self) -> usize {
        self.refused
            .iter()
            .filter(|(_, r)| *r == AdoptionRefusal::BareMinorDirective)
            .count()
    }
}

pub fn census<E: AdoptEnv>(env: &E, roots: &[String], fleet_go: &str) -> Census {
    let mut c = Census {
        scanned: roots.len(),
        ..Default::default()
    };
    for r in roots {
        match assess(env, r, fleet_go) {
            AdoptionVerdict::Eligible => c.eligible += 1,
            AdoptionVerdict::Refused(reason) => c.refused.push((r.clone(), reason)),
        }
    }
    c
}
