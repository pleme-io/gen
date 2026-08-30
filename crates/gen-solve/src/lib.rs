//! `gen-solve` — the fleet's version solver.
//!
//! ## Why this crate exists here
//!
//! Four modules of `gen-types` name "the resolver in `gen-engine`" as the thing
//! that does version-selection math against [`gen_types::VersionConstraint`].
//! `gen-engine` was never written. Meanwhile a real backtracking solver was
//! built in a leaf crate of `pleme-io/blue` — the fleet's only one, sitting one
//! level below the substrate that declares the capability missing.
//!
//! That is a **placement** problem, not a missing feature: the capability
//! exists and is in the wrong crate. This is it, moved to the layer that
//! already owns the vocabulary.
//!
//! ## It speaks gen-types, not a private dialect
//!
//! The input is `&[gen_types::Dependency]` — the exact shape `gen-cargo`,
//! `gen-npm`, `gen-bundler` and the rest already emit. There is no `Version`
//! and no `Range` defined here to keep in sync with the ones next door; the
//! only new type in the algebra is [`VersionSet`], and it exists because
//! [`gen_types::ConstraintSpec`] is a *syntax* and is not closed under
//! intersection. See [`set`] for why that distinction is load-bearing and why
//! this algebra is **not** a posture lattice's meet/join.
//!
//! ## What it guarantees, by tier
//!
//! Rounding these up would be the defect this crate is meant to remove, so they
//! are stated at the tier they actually reach:
//!
//! | Guarantee | Tier |
//! |---|---|
//! | An empty [`Interval`] has no inhabitant — `Interval::new` returns `None` | **truly-unrepresentable** (no public field access, no other constructor) |
//! | "Gave up" is never reported as "unsatisfiable" — [`SolveError`] has two arms | **truly-unrepresentable** (distinct variants; a caller cannot conflate them without matching) |
//! | A [`Conflict`] is explainable without parsing a string — [`PackageAt`] is typed | **truly-unrepresentable** (fields, not text) |
//! | The step bound is never exceeded | **only-mitigated** — a runtime check returning [`SolveError::Exhausted`]; `steps_taken() <= max_steps` is property-tested |
//! | A returned [`Resolution`] satisfies every constraint | **only-mitigated** — property-tested over generated graphs, not typed |
//! | Learning never changes the verdict, the picks, or the blamed package | **only-mitigated** — property-tested across both arms of [`Solver::without_learning`] |
//!
//! Note the third row's wording. Learning **can** change the requirement chain
//! carried by a [`Conflict`], and that is a limitation rather than a bug: a
//! chain is proved in the context that first established it, so a learned proof
//! may be a shorter — still true — witness than the one an unlearned search
//! would re-derive. [`Conflict::requirements`] is a *sufficient* explanation,
//! never a canonical one. Claiming otherwise would be rounding a tier up.
//!
//! ## Honest limits
//!
//! - **Not PubGrub.** See [`Solver`].
//! - **Nothing is fetched.** [`PackageUniverse`] is a trait with an in-memory
//!   implementation; a registry client would land behind it. Stated plainly so
//!   nobody reads a solve as an install.
//! - **Features and target predicates are not resolved.** [`gen_types::Dependency`]
//!   carries `features_enabled` and `target_predicate`; this solver ignores
//!   both. Feature unification is a second fixpoint on top of version
//!   selection, and half of it would silently resolve the wrong graph.
//!   [`Follow`] is the one edge filter, and it is explicit.
//!
//! ## Usage
//!
//! ```
//! use gen_solve::{edge, MapUniverse, Solver};
//! use gen_types::{ConstraintSpec, Version};
//!
//! let mut universe = MapUniverse::new();
//! universe
//!     .add("app", Version::new(1, 0, 0), vec![edge("lib", ConstraintSpec::Caret(Version::new(1, 0, 0)))])
//!     .add_leaf("lib", Version::new(1, 4, 0))
//!     .add_leaf("lib", Version::new(2, 0, 0));
//!
//! let resolved = Solver::new(&universe)
//!     .solve(&[edge("app", ConstraintSpec::Any)])
//!     .expect("resolves");
//! assert_eq!(resolved.picks["lib"], Version::new(1, 4, 0));
//! ```

pub mod set;
pub mod solver;
pub mod universe;

pub use set::{Bound, Interval, VersionSet, newest_first};
pub use solver::{
    Conflict, DEFAULT_MAX_STEPS, PackageAt, Requirement, Resolution, SolveError, Solver,
};
pub use universe::{Follow, MapUniverse, PackageUniverse, edge, edge_of_kind};
