//! Backtracking resolution with requirement-chain conflict attribution.

use crate::set::{VersionSet, newest_first};
use crate::universe::{Follow, PackageUniverse};
use core::fmt;
use gen_types::Version;
use std::collections::BTreeMap;

/// A concrete package coordinate: the thing that asked for something.
///
/// A **typed** pair, not a rendered `"name version"` string. Two reasons, both
/// load-bearing: a caller reading a [`Conflict`] gets fields to branch on
/// instead of a message to parse, and attribution (below) compares coordinates
/// structurally — a string compare would conflate a package literally named
/// `a 1.0.0` with the package `a` at version `1.0.0`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct PackageAt {
    pub name: String,
    pub version: Version,
}

impl fmt::Display for PackageAt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.name, self.version)
    }
}

/// One link in the chain of requirements that produced a constraint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Requirement {
    /// Who asked. `None` for the root.
    pub from: Option<PackageAt>,
    pub package: String,
    pub set: VersionSet,
}

impl fmt::Display for Requirement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.from {
            Some(from) => write!(f, "{from} requires {} {}", self.package, self.set),
            None => write!(f, "the root requires {} {}", self.package, self.set),
        }
    }
}

/// Why resolution failed, in enough detail to act on.
///
/// The contribution of a resolver is not the search — every resolver
/// backtracks newest-first. It is answering **why**: naming both sides and the
/// coordinate that failed. A bare "could not find compatible versions" is the
/// failure mode this type exists to make impossible, which is why it carries
/// the *chain* of requirements rather than a boolean.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict {
    /// The package nothing could be chosen for.
    pub package: String,
    /// Every requirement that bore on it. **Both sides**, so the reader does
    /// not have to reconstruct who disagreed.
    pub requirements: Vec<Requirement>,
    /// The intersection of those requirements. Empty when they contradict each
    /// other; non-empty when they agree and the registry simply has nothing in
    /// range — two different problems with two different fixes.
    pub demanded: VersionSet,
    /// What was published, so "never published" is distinguishable from
    /// "published, none satisfied".
    pub available: Vec<Version>,
}

impl Conflict {
    /// Do the requirements contradict each other (as opposed to agreeing on a
    /// range the registry cannot fill)?
    #[must_use]
    pub fn is_contradiction(&self) -> bool {
        self.demanded.is_empty()
    }
}

impl fmt::Display for Conflict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "cannot resolve `{}`:", self.package)?;
        for r in &self.requirements {
            writeln!(f, "  {r}")?;
        }
        if self.available.is_empty() {
            write!(f, "  and no versions of `{}` are published", self.package)
        } else {
            f.write_str("  available: ")?;
            for (i, v) in self.available.iter().enumerate() {
                if i > 0 {
                    f.write_str(", ")?;
                }
                write!(f, "{v}")?;
            }
            Ok(())
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SolveError {
    #[error("{0}")]
    Conflict(Conflict),
    /// The search hit its bound. Its own case because it is **not** proof that
    /// no solution exists — conflating the two would let a solver claim a graph
    /// is unsatisfiable when it merely gave up.
    #[error("resolution gave up after {steps} steps; this is not proof that no solution exists")]
    Exhausted { steps: usize },
}

/// The resolved set: one version per package reached.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Resolution {
    pub picks: BTreeMap<String, Version>,
}

impl fmt::Display for Resolution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, (name, version)) in self.picks.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            write!(f, "{name} {version}")?;
        }
        Ok(())
    }
}

/// Facts learned from failures: assignments that cannot be part of any
/// solution.
///
/// PubGrub's contribution in its smallest sound form. Without it a solver
/// re-derives the same dead end from every path that reaches it, and that is
/// where the exponential lives — the search is not exploring new ground, it is
/// re-proving something it already proved.
///
/// Deliberately narrow: concrete rejected assignments (`name@version`), not
/// general clauses over version sets. A full derivation graph subsumes more and
/// is a much larger thing to get right; this is the part that pays immediately,
/// and [`Solver::skipped_by_learning`] reports how much it caught so the value
/// is measured rather than assumed.
#[derive(Clone, Debug, Default)]
struct Learned {
    /// The dead assignment **and the conflict that proved it dead**.
    ///
    /// Keeping the proof rather than just the verdict is what lets a skip
    /// report a real failure instead of falling through to a synthesised
    /// root-chain conflict that blames the wrong package. That much IS held:
    /// which package gets blamed does not depend on how much of the search was
    /// skipped, and a property test pins it.
    ///
    /// What is **not** held — measured, not assumed — is chain-for-chain
    /// identity with an unlearned search. A requirement chain accumulates the
    /// requirements in scope where the failure was proved, so a proof
    /// established under a caller that had contributed fewer requirements is a
    /// shorter witness than the one a re-derivation would build. Both are true
    /// statements about the same failing package. Reproducing the longer chain
    /// would mean re-deriving it, which is the work this exists to skip.
    rejected: BTreeMap<PackageAt, SolveError>,
}

impl Learned {
    fn reason(&self, at: &PackageAt) -> Option<&SolveError> {
        self.rejected.get(at)
    }

    fn record(&mut self, at: PackageAt, why: SolveError) {
        self.rejected.entry(at).or_insert(why);
    }
}

/// Is this conflict caused by `at` **alone**?
///
/// Two conditions, and dropping either one is unsound:
///
/// 1. `at` must actually be **a party to the conflict** — it contributed at
///    least one requirement. A chain that is entirely root-origin proves
///    something about the *root*, not about `at`. Reading such a chain as "no
///    sibling contributed, therefore `at` is dead everywhere" condemns a
///    candidate that never participated, and because the condemnation then
///    propagates up, the search ends up blaming a package with no bearing on
///    the failure at all.
/// 2. **No third party** contributed. A sibling choice being implicated means
///    the candidate cannot be condemned on this evidence — it may be perfectly
///    fine under a different sibling. Condemning it anyway marks good
///    candidates dead, and with enough of them a satisfiable graph gets
///    reported as unsatisfiable: an optimisation that is not slower but
///    **wrong**.
///
/// Root requirements are exempt from (2) because they are invariant across the
/// whole search — they hold under every sibling choice, so they cannot be the
/// context that makes this candidate look worse than it is.
fn attributable_to(e: &SolveError, at: &PackageAt) -> bool {
    let SolveError::Conflict(c) = e else {
        return false;
    };
    c.requirements.iter().any(|r| r.from.as_ref() == Some(at))
        && c.requirements.iter().all(|r| match &r.from {
            None => true, // the root — invariant across the search
            Some(from) => from == at,
        })
}

/// Bound on the search. Generous for any real graph; finite so a pathological
/// one reports rather than hangs.
pub const DEFAULT_MAX_STEPS: usize = 100_000;

/// Newest-first backtracking resolution over a [`PackageUniverse`].
///
/// ## Honest limits
///
/// This is **backtracking with conflict attribution plus sound assignment-level
/// learning**, not PubGrub. PubGrub derives general incompatibilities over
/// version ranges and can attribute through several hops; this records only
/// concrete `name@version` rejections it can prove are context-independent. A
/// pathological graph can still take exponential time. [`Solver::steps_taken`]
/// reports the search size and [`SolveError::Exhausted`] fires at a bound, so
/// the difference is **visible rather than experienced as a hang**.
pub struct Solver<'u, U: PackageUniverse + ?Sized> {
    universe: &'u U,
    max_steps: usize,
    follow: Follow,
    learning: bool,
    steps: usize,
    learned: Learned,
    skipped: usize,
}

impl<'u, U: PackageUniverse + ?Sized> Solver<'u, U> {
    pub fn new(universe: &'u U) -> Self {
        Self {
            universe,
            max_steps: DEFAULT_MAX_STEPS,
            follow: Follow::default(),
            learning: true,
            steps: 0,
            learned: Learned::default(),
            skipped: 0,
        }
    }

    #[must_use]
    pub const fn with_max_steps(mut self, max_steps: usize) -> Self {
        self.max_steps = max_steps;
        self
    }

    #[must_use]
    pub const fn following(mut self, follow: Follow) -> Self {
        self.follow = follow;
        self
    }

    /// Turn learning off.
    ///
    /// Exists so "learning changes how long a solve takes and never what it
    /// returns" is a **testable** property rather than a claim in a comment: a
    /// property test runs both arms over the same graph and compares. A cache
    /// that alters answers is a bug wearing an optimisation's clothes, and
    /// without this switch nothing could catch one.
    #[must_use]
    pub const fn without_learning(mut self) -> Self {
        self.learning = false;
        self
    }

    /// How many candidate versions were attempted by the last solve.
    #[must_use]
    pub const fn steps_taken(&self) -> usize {
        self.steps
    }

    /// Candidate attempts skipped because the assignment was already proved
    /// dead. Zero on a graph with no repeated dead ends — which is most of
    /// them, and why this is reported rather than assumed.
    #[must_use]
    pub const fn skipped_by_learning(&self) -> usize {
        self.skipped
    }

    /// Resolve the root's requirements.
    ///
    /// The root is a plain slice of [`gen_types::Dependency`] — the same shape
    /// every gen adapter already emits — so a caller feeds `package.dependencies`
    /// straight in with no conversion step to keep in sync.
    pub fn solve(&mut self, root: &[gen_types::Dependency]) -> Result<Resolution, SolveError> {
        self.steps = 0;
        self.learned = Learned::default();
        self.skipped = 0;

        // Constraints accumulate as an intersection per package, alongside the
        // requirement chain that produced them — the chain is what makes a
        // conflict explainable.
        let mut constraints: BTreeMap<String, (VersionSet, Vec<Requirement>)> = BTreeMap::new();
        for dep in root.iter().filter(|d| self.follow.admits(d.kind)) {
            let set = VersionSet::of_constraint(&dep.constraint);
            let entry = constraints
                .entry(dep.name.clone())
                .or_insert_with(|| (VersionSet::any(), Vec::new()));
            entry.0 = entry.0.intersect(&set);
            entry.1.push(Requirement {
                from: None,
                package: dep.name.clone(),
                set,
            });
        }
        let mut picks = BTreeMap::new();
        self.search(&mut constraints, &mut picks)?;
        Ok(Resolution { picks })
    }

    fn conflict(
        &self,
        package: &str,
        requirements: Vec<Requirement>,
        demanded: VersionSet,
    ) -> SolveError {
        SolveError::Conflict(Conflict {
            package: package.to_string(),
            requirements,
            demanded,
            available: newest_first(self.universe.versions(package)),
        })
    }

    fn search(
        &mut self,
        constraints: &mut BTreeMap<String, (VersionSet, Vec<Requirement>)>,
        picks: &mut BTreeMap<String, Version>,
    ) -> Result<(), SolveError> {
        // Pick the next unresolved package deterministically (BTreeMap order),
        // so the same input always produces the same resolution AND the same
        // error. A resolver whose output depends on hash order is one nobody
        // can reproduce a bug in.
        let Some(name) = constraints
            .keys()
            .find(|name| !picks.contains_key(*name))
            .cloned()
        else {
            return Ok(()); // Everything resolved.
        };

        let (demanded, chain) = constraints[&name].clone();
        let available = newest_first(self.universe.versions(&name));
        let candidates: Vec<Version> = available
            .iter()
            .filter(|v| demanded.contains(v))
            .cloned()
            .collect();

        if candidates.is_empty() {
            return Err(self.conflict(&name, chain, demanded));
        }

        let mut last_conflict = None;
        for candidate in candidates {
            let at = PackageAt {
                name: name.clone(),
                version: candidate.clone(),
            };
            // Consult what previous failures proved. Reaching the same
            // assignment by a different path does not make it viable, and
            // re-deriving that is exactly the exponential.
            //
            // The stored proof becomes this branch's conflict. Without that,
            // a package whose every candidate is skipped falls through to the
            // synthesised root-chain conflict below and reports itself as the
            // culprit — so pruning would silently rewrite the diagnosis.
            if self.learning {
                if let Some(why) = self.learned.reason(&at).cloned() {
                    self.skipped += 1;
                    last_conflict = Some(why);
                    continue;
                }
            }
            // Checked BEFORE the increment, so `steps_taken() <= max_steps`
            // holds exactly. Incrementing first admits one attempt past the
            // bound and reports a count for work that was never done — an
            // off-by-one in the one number a caller uses to decide whether to
            // raise the bound or fix the graph.
            if self.steps >= self.max_steps {
                return Err(SolveError::Exhausted { steps: self.steps });
            }
            self.steps += 1;

            let Some(deps) = self.universe.dependencies(&name, &candidate) else {
                continue;
            };

            // Speculate on a copy, so a failed branch leaves nothing behind.
            // Without this, a rejected candidate's constraints poison the next
            // attempt and the solver reports a conflict that does not exist.
            let mut next_constraints = constraints.clone();
            let mut next_picks = picks.clone();
            next_picks.insert(name.clone(), candidate.clone());

            let mut viable = true;
            for dep in deps.iter().filter(|d| self.follow.admits(d.kind)) {
                let set = VersionSet::of_constraint(&dep.constraint);
                let entry = next_constraints
                    .entry(dep.name.clone())
                    .or_insert_with(|| (VersionSet::any(), Vec::new()));
                entry.0 = entry.0.intersect(&set);
                entry.1.push(Requirement {
                    from: Some(at.clone()),
                    package: dep.name.clone(),
                    set,
                });

                // Two ways this edge can be fatal: the demands now contradict,
                // or they still agree but rule out a version already chosen.
                // The second matters because a later requirement could
                // otherwise silently disagree with an earlier pick.
                let contradicts = entry.0.is_empty();
                let unpicks = next_picks
                    .get(&dep.name)
                    .is_some_and(|picked| !entry.0.contains(picked));
                if contradicts || unpicks {
                    last_conflict =
                        Some(self.conflict(&dep.name, entry.1.clone(), entry.0.clone()));
                    viable = false;
                    break;
                }
            }
            if !viable {
                // Learn ONLY if attributable — the same rule as the subtree
                // branch below, and unsound without it. The intersection is
                // taken against constraints ALREADY in scope, which include
                // every sibling the caller picked, so a candidate that fails
                // under `p@2` may be perfectly fine under `p@1`.
                if let Some(e) = &last_conflict {
                    if self.learning && attributable_to(e, &at) {
                        self.learned.record(at.clone(), e.clone());
                    }
                }
                continue;
            }

            match self.search(&mut next_constraints, &mut next_picks) {
                Ok(()) => {
                    *constraints = next_constraints;
                    *picks = next_picks;
                    return Ok(());
                }
                Err(e @ SolveError::Exhausted { .. }) => return Err(e),
                Err(e) => {
                    // "The subtree failed, so the assignment is dead" is FALSE:
                    // the subtree also carries constraints inherited from the
                    // caller, so it may have failed for a reason that has
                    // nothing to do with this candidate. See `attributable_to`.
                    if self.learning && attributable_to(&e, &at) {
                        self.learned.record(at.clone(), e.clone());
                    }
                    last_conflict = Some(e);
                }
            }
        }

        Err(last_conflict.unwrap_or_else(|| self.conflict(&name, chain, demanded)))
    }
}
