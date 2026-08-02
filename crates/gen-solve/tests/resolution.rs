//! The behavioural corpus: what resolution must do, and what a failure must
//! say.
//!
//! Every test here names the defect it guards. A resolver test suite that only
//! asserts "the happy graph resolves" grades itself on the easy half — the
//! search is the easy half.

use gen_solve::{edge, edge_of_kind, Conflict, Follow, MapUniverse, SolveError, Solver, VersionSet};
use gen_types::{
    Combinator, CompoundConstraint, ConstraintSpec, Dependency, DependencyKind, Version,
    VersionConstraint,
};

fn v(a: u64, b: u64, c: u64) -> Version {
    Version::new(a, b, c)
}

fn caret(a: u64, b: u64, c: u64) -> ConstraintSpec {
    ConstraintSpec::Caret(v(a, b, c))
}

// ---------------------------------------------------------------------------
// Resolution
// ---------------------------------------------------------------------------

#[test]
fn a_single_dependency_resolves_to_the_newest_match() {
    let mut u = MapUniverse::new();
    u.add_leaf("a", v(1, 0, 0))
        .add_leaf("a", v(1, 5, 0))
        .add_leaf("a", v(2, 0, 0));
    let out = Solver::new(&u)
        .solve(&[edge("a", caret(1, 0, 0))])
        .expect("resolves");
    assert_eq!(out.picks["a"], v(1, 5, 0), "newest within the range");
}

#[test]
fn transitive_dependencies_resolve() {
    let mut u = MapUniverse::new();
    u.add("a", v(1, 0, 0), vec![edge("b", caret(1, 0, 0))])
        .add(
            "b",
            v(1, 2, 0),
            vec![edge("c", ConstraintSpec::GreaterEqual(v(0, 1, 0)))],
        )
        .add_leaf("c", v(0, 4, 0));
    let out = Solver::new(&u)
        .solve(&[edge("a", caret(1, 0, 0))])
        .expect("resolves");
    assert_eq!(out.picks.len(), 3);
    assert_eq!(out.picks["c"], v(0, 4, 0));
}

/// **Backtracking.** The newest `a` needs a `b` that does not exist, so the
/// solver must fall back to an older `a` rather than reporting failure.
#[test]
fn the_solver_backtracks_past_an_unsatisfiable_newest_version() {
    let mut u = MapUniverse::new();
    u.add("a", v(2, 0, 0), vec![edge("b", caret(9, 0, 0))])
        .add("a", v(1, 0, 0), vec![edge("b", caret(1, 0, 0))])
        .add_leaf("b", v(1, 0, 0));
    let out = Solver::new(&u)
        .solve(&[edge("a", ConstraintSpec::Any)])
        .expect("must backtrack");
    assert_eq!(out.picks["a"], v(1, 0, 0), "a 2.0.0 is unsatisfiable");
    assert_eq!(out.picks["b"], v(1, 0, 0));
}

/// A failed branch must leave nothing behind. Without speculating on a copy the
/// rejected candidate's constraints poison the retry, and the solver reports a
/// conflict that does not exist.
#[test]
fn a_rejected_candidate_does_not_poison_the_retry() {
    let mut u = MapUniverse::new();
    u.add("a", v(2, 0, 0), vec![edge("shared", caret(2, 0, 0))])
        .add("a", v(1, 0, 0), vec![edge("shared", caret(1, 0, 0))])
        .add_leaf("shared", v(1, 0, 0));
    // The root also pins `shared` to 1.x, so a@2.0.0 must fail and a@1.0.0 must
    // then succeed against an UNPOLLUTED constraint set.
    let out = Solver::new(&u)
        .solve(&[
            edge("a", ConstraintSpec::Any),
            edge("shared", caret(1, 0, 0)),
        ])
        .expect("a@1.0.0 must still resolve");
    assert_eq!(out.picks["a"], v(1, 0, 0));
    assert_eq!(out.picks["shared"], v(1, 0, 0));
}

/// A diamond — two paths to one package — resolves to ONE version, not two.
#[test]
fn a_diamond_resolves_to_a_single_shared_version() {
    let mut u = MapUniverse::new();
    u.add(
        "left",
        v(1, 0, 0),
        vec![edge("shared", ConstraintSpec::GreaterEqual(v(1, 0, 0)))],
    )
    .add("right", v(1, 0, 0), vec![edge("shared", caret(1, 2, 0))])
    .add_leaf("shared", v(1, 0, 0))
    .add_leaf("shared", v(1, 5, 0))
    .add_leaf("shared", v(2, 0, 0));
    let out = Solver::new(&u)
        .solve(&[
            edge("left", ConstraintSpec::Any),
            edge("right", ConstraintSpec::Any),
        ])
        .expect("resolves");
    assert_eq!(
        out.picks["shared"],
        v(1, 5, 0),
        "the intersection of >=1.0.0 and ^1.2.0, newest first"
    );
}

#[test]
fn an_empty_root_resolves_to_an_empty_set() {
    let u = MapUniverse::new();
    let out = Solver::new(&u).solve(&[]).expect("resolves");
    assert!(out.picks.is_empty());
}

/// **Two root edges on one package are conjoined, not last-one-wins.** A root
/// that declares `>=1.2` and `<2.0` separately (the shape pip and npm both
/// produce) must get the intersection.
#[test]
fn repeated_root_edges_on_one_package_intersect() {
    let mut u = MapUniverse::new();
    u.add_leaf("a", v(1, 1, 0))
        .add_leaf("a", v(1, 5, 0))
        .add_leaf("a", v(2, 5, 0));
    let out = Solver::new(&u)
        .solve(&[
            edge("a", ConstraintSpec::GreaterEqual(v(1, 2, 0))),
            edge("a", ConstraintSpec::Less(v(2, 0, 0))),
        ])
        .expect("resolves");
    assert_eq!(out.picks["a"], v(1, 5, 0));
}

// ---------------------------------------------------------------------------
// What a failure says
// ---------------------------------------------------------------------------

fn conflict_of(e: SolveError) -> Conflict {
    match e {
        SolveError::Conflict(c) => c,
        other => panic!("expected a Conflict, got {other:?}"),
    }
}

/// **A conflict names both sides and the coordinate.** This is the whole point:
/// "could not find compatible versions" is the message this crate exists to
/// avoid.
#[test]
fn a_conflict_names_both_requirements_and_what_was_available() {
    let mut u = MapUniverse::new();
    u.add("left", v(1, 0, 0), vec![edge("shared", caret(1, 0, 0))])
        .add("right", v(1, 0, 0), vec![edge("shared", caret(2, 0, 0))])
        .add_leaf("shared", v(1, 0, 0))
        .add_leaf("shared", v(2, 0, 0));
    let err = Solver::new(&u)
        .solve(&[
            edge("left", ConstraintSpec::Any),
            edge("right", ConstraintSpec::Any),
        ])
        .expect_err("1.x and 2.x cannot both hold");
    let c = conflict_of(err);
    assert_eq!(c.package, "shared");

    // The typed surface, first: a caller must be able to act on this WITHOUT
    // reading the rendered message.
    let culprits: Vec<String> = c
        .requirements
        .iter()
        .filter_map(|r| r.from.as_ref().map(|f| f.name.clone()))
        .collect();
    assert!(culprits.contains(&"left".to_string()), "{culprits:?}");
    assert!(culprits.contains(&"right".to_string()), "{culprits:?}");
    assert!(c.is_contradiction(), "the two demands genuinely contradict");

    // And the rendering carries the same facts.
    let text = c.to_string();
    for fragment in ["left", "right", "1.0.0", "2.0.0"] {
        assert!(text.contains(fragment), "must name {fragment}: {text}");
    }
}

/// "Never published" and "published but none satisfied" are different problems
/// with different fixes, so they read differently.
#[test]
fn an_unpublished_package_is_distinguishable_from_an_unsatisfiable_one() {
    let u = MapUniverse::new();
    let err = Solver::new(&u)
        .solve(&[edge("ghost", ConstraintSpec::Any)])
        .expect_err("no versions");
    assert!(
        err.to_string()
            .contains("no versions of `ghost` are published"),
        "got {err}"
    );

    let mut u2 = MapUniverse::new();
    u2.add_leaf("real", v(1, 0, 0));
    let err2 = Solver::new(&u2)
        .solve(&[edge("real", caret(5, 0, 0))])
        .expect_err("no match");
    let text = err2.to_string();
    assert!(!text.contains("no versions"), "it IS published: {text}");
    assert!(text.contains("available: 1.0.0"), "{text}");
}

/// **A contradiction and an unfillable-but-coherent demand are different
/// facts.** Both fail; only one is fixed by publishing a version. `demanded`
/// carries the distinction so a caller can say which.
#[test]
fn a_coherent_demand_the_registry_cannot_fill_is_not_a_contradiction() {
    let mut u = MapUniverse::new();
    u.add_leaf("a", v(1, 0, 0));
    let c = conflict_of(
        Solver::new(&u)
            .solve(&[edge("a", caret(5, 0, 0))])
            .expect_err("nothing in range"),
    );
    assert!(
        !c.is_contradiction(),
        "^5.0.0 alone is perfectly coherent; the registry just lacks it"
    );
    assert!(c.demanded.contains(&v(5, 1, 0)), "{}", c.demanded);
}

/// **Giving up is not proof of unsatisfiability.** Conflating the two lets a
/// solver claim a graph is impossible when it merely ran out of budget.
#[test]
fn exhaustion_is_reported_as_its_own_case_not_as_a_conflict() {
    let mut u = MapUniverse::new();
    for patch in 0..20 {
        u.add("a", v(1, 0, patch), vec![edge("b", caret(9, 0, 0))]);
    }
    u.add_leaf("b", v(1, 0, 0));
    let err = Solver::new(&u)
        .with_max_steps(1)
        .solve(&[edge("a", ConstraintSpec::Any)])
        .expect_err("must give up");
    assert!(
        matches!(err, SolveError::Exhausted { .. }),
        "expected Exhausted, got {err:?}"
    );
    assert!(
        err.to_string().contains("not proof"),
        "the message must not read as unsatisfiability: {err}"
    );
}

/// Anti-vacuity for the test above: the same graph with a real budget resolves,
/// so the exhaustion is the budget's doing and not a hidden conflict.
#[test]
fn the_same_graph_resolves_with_a_normal_budget() {
    let mut u = MapUniverse::new();
    for patch in 0..20 {
        u.add("a", v(1, 0, patch), vec![edge("b", caret(9, 0, 0))]);
    }
    u.add_leaf("a", v(0, 9, 0)).add_leaf("b", v(1, 0, 0));
    let out = Solver::new(&u)
        .solve(&[edge("a", ConstraintSpec::Any)])
        .expect("resolves with a real budget");
    assert_eq!(out.picks["a"], v(0, 9, 0));
}

/// **Determinism.** Same input, same resolution AND same error — a resolver
/// whose output depends on iteration order is one nobody can reproduce a bug in.
#[test]
fn resolution_is_deterministic() {
    let mut u = MapUniverse::new();
    u.add("a", v(1, 0, 0), vec![edge("c", ConstraintSpec::Any)])
        .add("b", v(1, 0, 0), vec![edge("c", ConstraintSpec::Any)])
        .add_leaf("c", v(1, 0, 0))
        .add_leaf("c", v(1, 1, 0));
    let root = [
        edge("a", ConstraintSpec::Any),
        edge("b", ConstraintSpec::Any),
    ];
    let first = Solver::new(&u).solve(&root).expect("resolves");
    for _ in 0..8 {
        assert_eq!(Solver::new(&u).solve(&root).expect("resolves"), first);
    }
}

// ---------------------------------------------------------------------------
// Edge selection
// ---------------------------------------------------------------------------

/// **`Follow::All` is the default, and it does not silently drop a dev edge.**
/// Narrowing the graph can turn an unsatisfiable manifest into a satisfiable
/// resolve — the failure you then meet at install time.
#[test]
fn the_default_follows_every_declared_edge() {
    let mut u = MapUniverse::new();
    u.add(
        "app",
        v(1, 0, 0),
        vec![edge_of_kind("only-dev", caret(9, 0, 0), DependencyKind::Dev)],
    );
    let root = [edge("app", ConstraintSpec::Any)];

    let err = Solver::new(&u)
        .solve(&root)
        .expect_err("the dev edge is unsatisfiable and must not be skipped");
    assert_eq!(conflict_of(err).package, "only-dev");

    let out = Solver::new(&u)
        .following(Follow::Runtime)
        .solve(&root)
        .expect("Runtime skips the dev edge — explicitly, at the caller's word");
    assert_eq!(out.picks.len(), 1);
    assert!(!out.picks.contains_key("only-dev"));
}

// ---------------------------------------------------------------------------
// Learning: it must save work and never change an answer
// ---------------------------------------------------------------------------

/// A graph that forces the SAME dead end to be reached from many paths.
///
/// `n` independent packages named `aaa*` each have two versions and no edges,
/// so the solver enumerates 2^n combinations of them. `zzz` sorts LAST, so
/// every one of those combinations reaches it — and every `zzz` version needs a
/// `ghost` that does not exist.
///
/// The names matter: selection is BTreeMap order, so a dead end sorting
/// *before* the free packages would be reached once, learned, and never
/// revisited — and the test would measure nothing. Repetition has to be
/// constructed, not assumed.
fn repeated_dead_end(n: usize) -> (MapUniverse, Vec<Dependency>) {
    let mut u = MapUniverse::new();
    let mut root = Vec::new();
    for i in 0..n {
        let name = ["aaa", &i.to_string()].concat();
        u.add_leaf(&name, v(1, 0, 0)).add_leaf(&name, v(2, 0, 0));
        root.push(edge(&name, ConstraintSpec::Any));
    }
    for patch in 0..6 {
        u.add("zzz", v(1, 0, patch), vec![edge("ghost", caret(9, 0, 0))]);
    }
    root.push(edge("zzz", ConstraintSpec::Any));
    (u, root)
}

/// **Learning proves the dead end once and skips it thereafter.**
#[test]
fn learning_skips_a_dead_end_already_proved() {
    let (u, root) = repeated_dead_end(5);
    let mut solver = Solver::new(&u);
    solver.solve(&root).expect_err("the graph is unsatisfiable");
    assert!(
        solver.skipped_by_learning() >= 6,
        "the dead end must be re-reached and skipped, not re-derived; \
         skipped={} steps={}",
        solver.skipped_by_learning(),
        solver.steps_taken()
    );
}

/// **The dead end costs a CONSTANT, not a multiple.** Doubling the free
/// packages multiplies the combinations — that work is real. What must not
/// multiply is the dead end.
#[test]
fn the_dead_end_is_paid_for_once_regardless_of_branching() {
    let mut measured = Vec::new();
    for n in [3usize, 6] {
        let (u, root) = repeated_dead_end(n);
        let mut solver = Solver::new(&u);
        solver.solve(&root).expect_err("unsatisfiable");
        measured.push((solver.steps_taken(), solver.skipped_by_learning()));
    }
    let (small_steps, small_skipped) = measured[0];
    let (large_steps, large_skipped) = measured[1];
    assert!(
        large_skipped > small_skipped,
        "more branching must mean more skips, not more work: {measured:?}"
    );
    assert!(
        large_steps - small_steps < large_skipped,
        "the growth in real work must be smaller than what learning avoided: {measured:?}"
    );
}

/// **Turning learning off must cost work and change nothing else.** This is the
/// anti-vacuity check for `without_learning` itself: if the switch did nothing,
/// every property test that compares the two arms would be comparing one arm to
/// itself.
#[test]
fn disabling_learning_costs_more_steps_on_a_repeated_dead_end() {
    let (u, root) = repeated_dead_end(5);
    let mut on = Solver::new(&u);
    let mut off = Solver::new(&u).without_learning();
    let with = on.solve(&root).expect_err("unsatisfiable");
    let without = off.solve(&root).expect_err("unsatisfiable");
    assert_eq!(with, without, "the switch must not change the answer");
    assert!(
        off.steps_taken() > on.steps_taken(),
        "learning off must attempt strictly more candidates: on={} off={}",
        on.steps_taken(),
        off.steps_taken()
    );
    assert_eq!(off.skipped_by_learning(), 0);
}

/// **A conflict must still be reported, and still name both sides.** Learning
/// prunes the search; it must not prune the explanation.
#[test]
fn learning_does_not_degrade_the_conflict_report() {
    let mut u = MapUniverse::new();
    u.add("left", v(1, 0, 0), vec![edge("shared", caret(1, 0, 0))])
        .add("right", v(1, 0, 0), vec![edge("shared", caret(2, 0, 0))])
        .add_leaf("shared", v(1, 0, 0))
        .add_leaf("shared", v(2, 0, 0));
    let err = Solver::new(&u)
        .solve(&[
            edge("left", ConstraintSpec::Any),
            edge("right", ConstraintSpec::Any),
        ])
        .expect_err("unsatisfiable");
    let text = err.to_string();
    assert!(text.contains("left") && text.contains("right"), "{text}");
}

/// Learned facts are per-solve. A `Solver` reused must not carry stale ones.
#[test]
fn learned_facts_do_not_leak_between_solves() {
    let mut u = MapUniverse::new();
    u.add("a", v(1, 0, 0), vec![edge("ghost", ConstraintSpec::Any)]);
    let mut solver = Solver::new(&u);
    assert!(solver.solve(&[edge("a", ConstraintSpec::Any)]).is_err());

    let mut u2 = MapUniverse::new();
    u2.add_leaf("a", v(1, 0, 0));
    let mut solver2 = Solver::new(&u2);
    assert!(solver2.solve(&[edge("a", ConstraintSpec::Any)]).is_ok());
    assert_eq!(solver2.skipped_by_learning(), 0);
}

// ---------------------------------------------------------------------------
// Soundness of learning — the two graphs that catch it being wrong
// ---------------------------------------------------------------------------

/// **A failure in ONE caller context must not condemn the candidate in
/// another.**
///
/// `p` is chosen before `q`. Under `p@2` the shared constraint narrows to `^2`,
/// so `q@1`'s own `^1` demand intersects to empty and `q@1` fails — *in that
/// context*. Under `p@1` it is fine. A solver that condemns `q@1` on the first
/// failure reports an **unsatisfiable graph that is satisfiable**: not slower,
/// WRONG.
#[test]
fn a_context_dependent_failure_does_not_condemn_the_candidate() {
    let mut u = MapUniverse::new();
    u.add("p", v(2, 0, 0), vec![edge("shared", caret(2, 0, 0))])
        .add("p", v(1, 0, 0), vec![edge("shared", caret(1, 0, 0))])
        .add("q", v(1, 0, 0), vec![edge("shared", caret(1, 0, 0))])
        .add_leaf("shared", v(1, 0, 0))
        .add_leaf("shared", v(2, 0, 0));
    let out = Solver::new(&u)
        .solve(&[
            edge("p", ConstraintSpec::Any),
            edge("q", ConstraintSpec::Any),
        ])
        .expect("this graph IS satisfiable: p@1 + q@1 + shared@1");
    assert_eq!(out.picks["p"], v(1, 0, 0));
    assert_eq!(out.picks["q"], v(1, 0, 0));
    assert_eq!(out.picks["shared"], v(1, 0, 0));
}

/// **The same trap one level deeper — a context-dependent SUBTREE failure.**
///
/// A separate gate because it exercises a separate branch: condemning on
/// subtree failure and condemning on empty intersection are two different
/// pieces of code, and a test for one does not cover the other.
#[test]
fn a_context_dependent_subtree_failure_does_not_condemn_the_candidate() {
    let mut u = MapUniverse::new();
    u.add("p", v(2, 0, 0), vec![edge("shared", caret(2, 0, 0))])
        .add("p", v(1, 0, 0), vec![edge("shared", caret(1, 0, 0))])
        .add("q", v(1, 0, 0), vec![edge("mid", caret(1, 0, 0))])
        .add("mid", v(1, 0, 0), vec![edge("shared", caret(1, 0, 0))])
        .add_leaf("shared", v(1, 0, 0))
        .add_leaf("shared", v(2, 0, 0));
    let out = Solver::new(&u)
        .solve(&[
            edge("p", ConstraintSpec::Any),
            edge("q", ConstraintSpec::Any),
        ])
        .expect("satisfiable: p@1 + q@1 + mid@1 + shared@1");
    assert_eq!(out.picks["p"], v(1, 0, 0));
    assert_eq!(out.picks["q"], v(1, 0, 0));
    assert_eq!(out.picks["mid"], v(1, 0, 0));
}

// ---------------------------------------------------------------------------
// The set algebra, checked against gen-types' own predicate
// ---------------------------------------------------------------------------

/// **`VersionSet` must agree with `ConstraintSpec::matches`, which is an
/// independent implementation.** A lowering checked against itself is a
/// tautology; checked against the predicate every adapter already trusts, it is
/// evidence.
#[test]
fn the_lowering_agrees_with_gen_types_matcher_on_every_spec_variant() {
    let specs = [
        ConstraintSpec::Exact(v(1, 2, 3)),
        ConstraintSpec::Range {
            lower_inclusive: v(1, 0, 0),
            upper_exclusive: v(2, 0, 0),
        },
        ConstraintSpec::Tilde(v(1, 2, 3)),
        ConstraintSpec::Caret(v(1, 2, 3)),
        ConstraintSpec::Caret(v(0, 2, 3)),
        ConstraintSpec::Caret(v(0, 0, 3)),
        ConstraintSpec::GreaterEqual(v(1, 0, 0)),
        ConstraintSpec::Greater(v(1, 0, 0)),
        ConstraintSpec::LessEqual(v(1, 0, 0)),
        ConstraintSpec::Less(v(1, 0, 0)),
        ConstraintSpec::Any,
    ];
    let probes = [
        v(0, 0, 3),
        v(0, 0, 4),
        v(0, 2, 3),
        v(0, 3, 0),
        v(1, 0, 0),
        v(1, 2, 2),
        v(1, 2, 3),
        v(1, 2, 99),
        v(1, 3, 0),
        v(2, 0, 0),
        v(99, 0, 0),
    ];
    for spec in &specs {
        let lowered = VersionSet::of_spec(spec);
        let native = VersionConstraint::from_spec(spec.clone());
        for probe in &probes {
            assert_eq!(
                lowered.contains(probe),
                native.matches(probe),
                "{spec:?} disagrees at {probe}: lowered={lowered}"
            );
        }
    }
}

/// The same agreement for a disjunction, which is the case a conjunction-only
/// algebra would get wrong.
///
/// **`of_compound` is not reachable from `Solver::solve` today**, and this test
/// exercises it directly rather than through a `Dependency` because it cannot
/// be reached that way: `Dependency` holds one `VersionConstraint`, which holds
/// exactly one atomic `ConstraintSpec`. A conjunction still reaches the solver
/// — as two `Dependency` entries on one name, which intersect — but a
/// disjunction has no path. The union algebra is built, correct and tested;
/// wiring it up needs a compound-carrying edge in `gen-types`, which is that
/// crate's call to make.
#[test]
fn the_lowering_agrees_with_gen_types_matcher_on_a_disjunction() {
    let compound = CompoundConstraint {
        combinator: Combinator::Or,
        atoms: vec![caret(1, 0, 0), caret(3, 0, 0)],
    };
    let lowered = VersionSet::of_compound(&compound);
    for probe in [v(0, 9, 0), v(1, 0, 0), v(2, 0, 0), v(3, 5, 0), v(4, 0, 0)] {
        assert_eq!(
            lowered.contains(&probe),
            compound.matches(&probe),
            "disagreement at {probe}: lowered={lowered}"
        );
    }
}
