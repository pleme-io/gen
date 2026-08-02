//! The invariants that are genuinely properties — true of *every* graph, not
//! of the handful anyone thought to write down.
//!
//! `tests/resolution.rs` holds the behavioural corpus: each test there names a
//! specific defect and pins a specific answer. That is the right shape for
//! "a conflict must name both sides". It is the wrong shape for "the answer is
//! correct", because a resolver's interesting failures live in the graphs
//! nobody imagined. The four properties here are the ones whose statement is
//! genuinely universally quantified:
//!
//! 1. a returned [`Resolution`] satisfies **every** constraint that bore on it;
//! 2. the step bound is **never** exceeded;
//! 3. learning changes how long a solve takes and **never** what it returns;
//! 4. the [`VersionSet`] algebra is **pointwise** — intersection is `&&` and
//!    union is `||` over `contains`, at every version.
//!
//! Property 1 is the load-bearing one. Everything else the crate does is in
//! service of producing an answer, and an answer that violates a declared
//! constraint is the one bug that makes the whole crate worthless — while being
//! exactly the bug a hand-written fixture is least likely to catch, because you
//! have to already suspect the graph shape to write it down.

use gen_solve::{edge, MapUniverse, PackageUniverse, Resolution, SolveError, Solver, VersionSet};
use gen_types::{ConstraintSpec, Dependency, Version};
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// Generators
// ---------------------------------------------------------------------------

/// A small version pool. Deliberately tiny: collisions between packages'
/// version numbers are what force intersections to actually bite, and a wide
/// random range would make almost every graph trivially satisfiable.
fn a_version() -> impl Strategy<Value = Version> {
    (0u64..3, 0u64..3).prop_map(|(major, minor)| Version::new(major, minor, 0))
}

/// The spec variants a real manifest actually carries, across the whole
/// interesting shape space: unbounded, closed, half-open both ways, and a
/// point.
fn a_spec() -> impl Strategy<Value = ConstraintSpec> {
    prop_oneof![
        Just(ConstraintSpec::Any),
        a_version().prop_map(ConstraintSpec::Exact),
        a_version().prop_map(ConstraintSpec::Caret),
        a_version().prop_map(ConstraintSpec::Tilde),
        a_version().prop_map(ConstraintSpec::GreaterEqual),
        a_version().prop_map(ConstraintSpec::Greater),
        a_version().prop_map(ConstraintSpec::LessEqual),
        a_version().prop_map(ConstraintSpec::Less),
    ]
}

fn name_of(i: usize) -> String {
    ["p", &i.to_string()].concat()
}

/// A random package graph plus a random root.
///
/// Edges may point at **any** package, including backwards and at the package
/// itself. An earlier draft restricted them to point forward — which yields an
/// acyclic graph and reads like a simplification with no cost. It is not.
/// Packages are selected in `BTreeMap` order, so with forward-only edges every
/// requirement on a package necessarily arrives *before* that package is
/// picked, and the solver's "a later requirement rules out a version already
/// chosen" branch becomes unreachable. A deliberate mutation of that branch was
/// not caught until this restriction came off — the generator was quietly
/// grading the solver on a subset of its own logic. Cycles are fine: a package
/// is picked at most once, so the search still terminates, and the oracle never
/// needed acyclicity in the first place.
fn a_graph() -> impl Strategy<Value = (MapUniverse, Vec<Dependency>)> {
    (2usize..5).prop_flat_map(|package_count| {
        // For each package: its versions, and for each version its edges.
        let packages = (0..package_count)
            .map(|_| {
                proptest::collection::vec(
                    (
                        a_version(),
                        proptest::collection::vec((0..package_count, a_spec()), 0..3),
                    ),
                    1..3,
                )
            })
            .collect::<Vec<_>>();
        let root = proptest::collection::vec((0..package_count, a_spec()), 1..4);
        (packages, root).prop_map(move |(packages, root)| {
            let mut universe = MapUniverse::new();
            for (i, versions) in packages.into_iter().enumerate() {
                for (version, edges) in versions {
                    let deps = edges
                        .into_iter()
                        .map(|(target, spec)| edge(&name_of(target), spec))
                        .collect();
                    universe.add(&name_of(i), version, deps);
                }
            }
            let root = root
                .into_iter()
                .map(|(target, spec)| edge(&name_of(target), spec))
                .collect();
            (universe, root)
        })
    })
}

// ---------------------------------------------------------------------------
// The oracle
// ---------------------------------------------------------------------------

/// Check a resolution against the graph that produced it, independently of the
/// solver.
///
/// Deliberately written with `VersionConstraint::matches` — gen-types' own
/// predicate — rather than with [`VersionSet`]. Checking the solver's answer
/// with the solver's own algebra would pass even if the lowering were wrong;
/// checking it against the matcher every adapter already trusts is evidence
/// about both at once.
fn violation(
    universe: &MapUniverse,
    root: &[Dependency],
    resolved: &Resolution,
) -> Option<String> {
    // Every requirement in force: the root's, plus those of each picked
    // package at its picked version.
    let mut requirements: Vec<(String, &Dependency)> =
        root.iter().map(|d| ("the root".to_string(), d)).collect();
    let owned: Vec<(String, Vec<Dependency>)> = resolved
        .picks
        .iter()
        .map(|(name, version)| {
            (
                [name.as_str(), " ", &version.to_string()].concat(),
                universe.dependencies(name, version).unwrap_or_default(),
            )
        })
        .collect();
    for (who, deps) in &owned {
        requirements.extend(deps.iter().map(|d| (who.clone(), d)));
    }

    for (who, dep) in requirements {
        let Some(picked) = resolved.picks.get(&dep.name) else {
            return Some(
                [
                    &who,
                    " requires ",
                    &dep.name,
                    ", which is absent from the resolution",
                ]
                .concat(),
            );
        };
        if !dep.constraint.matches(picked) {
            return Some(
                [
                    &who,
                    " requires ",
                    &dep.name,
                    " ",
                    // `ConstraintSpec` has no `Display`; the lowered set does,
                    // and it names the same versions.
                    &VersionSet::of_constraint(&dep.constraint).to_string(),
                    ", but ",
                    &picked.to_string(),
                    " was picked",
                ]
                .concat(),
            );
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Properties
// ---------------------------------------------------------------------------

proptest! {
    /// **A returned resolution satisfies every constraint that bore on it.**
    ///
    /// Both directions of "bore on it": the root's own edges, and the edges of
    /// every package the solver chose. The second half is what catches a
    /// solver that picks a version and then forgets to honour what that
    /// version itself demands.
    ///
    /// Failures are not counted — an unsatisfiable graph is a legitimate answer
    /// and the generator produces plenty. The claim is conditional: *if* it
    /// returns a resolution, that resolution is correct.
    #[test]
    fn a_returned_resolution_satisfies_every_constraint((universe, root) in a_graph()) {
        if let Ok(resolved) = Solver::new(&universe).solve(&root) {
            prop_assert!(
                violation(&universe, &root, &resolved).is_none(),
                "{}",
                violation(&universe, &root, &resolved).unwrap_or_default()
            );
        }
    }

    /// **The step bound is never exceeded.**
    ///
    /// Checked at a bound low enough that real graphs hit it, so the property
    /// is exercised rather than vacuously satisfied by graphs that finish
    /// early. Both outcomes are constrained: a solve that gives up must report
    /// `Exhausted`, and either way the counter must respect the bound it was
    /// given.
    #[test]
    fn the_step_bound_is_never_exceeded((universe, root) in a_graph(), bound in 1usize..12) {
        let mut solver = Solver::new(&universe).with_max_steps(bound);
        let outcome = solver.solve(&root);
        prop_assert!(
            solver.steps_taken() <= bound,
            "took {} steps against a bound of {bound}",
            solver.steps_taken()
        );
        if let Err(SolveError::Exhausted { steps }) = outcome {
            prop_assert!(steps <= bound, "reported {steps} steps against a bound of {bound}");
        }
    }

    /// **Learning never changes the answer** — where "the answer" is whether
    /// the graph resolves, what it resolves to, and which package is blamed.
    ///
    /// The unsound-learning failure mode is specifically that a *satisfiable*
    /// graph starts reporting as unsatisfiable, so comparing the two arms over
    /// random graphs is the check that catches it.
    ///
    /// **The conflict payload is deliberately NOT compared, and that is a real
    /// limitation rather than a test convenience.** A `Conflict` carries the
    /// requirement chain *in scope where the failure was proved*, and learning
    /// replays a proof from the context that first established it. Both chains
    /// are true statements about the same failing package; the learned one can
    /// simply be shorter, having been proved under a caller that had
    /// contributed fewer requirements. Reproducing the longer chain would mean
    /// re-deriving it — which is precisely the work learning exists to skip.
    /// So the honest guarantee is: same verdict, same picks, same culprit;
    /// possibly a smaller witness. `SolveError::Conflict::requirements` is a
    /// *sufficient* explanation, never a canonical one.
    #[test]
    fn learning_never_changes_the_answer((universe, root) in a_graph()) {
        let with = Solver::new(&universe).solve(&root);
        let without = Solver::new(&universe).without_learning().solve(&root);
        match (&with, &without) {
            (Ok(a), Ok(b)) => prop_assert_eq!(a, b, "learning changed the resolution"),
            (Err(SolveError::Conflict(a)), Err(SolveError::Conflict(b))) => {
                prop_assert_eq!(
                    &a.package, &b.package,
                    "learning changed which package is blamed"
                );
                prop_assert_eq!(
                    a.is_contradiction(), b.is_contradiction(),
                    "learning changed contradiction-vs-unfillable for `{}`", &a.package
                );
            }
            (Err(SolveError::Exhausted { .. }), Err(SolveError::Exhausted { .. })) => {}
            _ => prop_assert!(
                false,
                "learning changed the verdict: with={:?} without={:?}", with, without
            ),
        }
    }

    /// **Intersection is pointwise conjunction and union is pointwise
    /// disjunction**, at every version — which is what makes the interval
    /// representation an *implementation* of set algebra rather than an
    /// approximation of one.
    #[test]
    fn the_algebra_is_pointwise(a in a_spec(), b in a_spec(), probe in a_version()) {
        let (sa, sb) = (VersionSet::of_spec(&a), VersionSet::of_spec(&b));
        prop_assert_eq!(
            sa.intersect(&sb).contains(&probe),
            sa.contains(&probe) && sb.contains(&probe),
            "intersection disagrees at {} for {} ∩ {}", probe, sa, sb
        );
        prop_assert_eq!(
            sa.union(&sb).contains(&probe),
            sa.contains(&probe) || sb.contains(&probe),
            "union disagrees at {} for {} ∪ {}", probe, sa, sb
        );
    }

    /// **The lowering agrees with gen-types' own matcher on every spec.**
    ///
    /// The fixture version of this test walks a hand-picked probe list; this
    /// one walks arbitrary ones. If `VersionSet::of_spec` and
    /// `ConstraintSpec::matches` ever disagree, every conflict report the
    /// solver produces is describing a different constraint than the one the
    /// adapter parsed.
    #[test]
    fn the_lowering_agrees_with_the_matcher(spec in a_spec(), probe in a_version()) {
        prop_assert_eq!(
            VersionSet::of_spec(&spec).contains(&probe),
            gen_types::VersionConstraint::from_spec(spec.clone()).matches(&probe),
            "{:?} disagrees at {}", spec, probe
        );
    }
}
