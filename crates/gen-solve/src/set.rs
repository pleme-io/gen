//! [`VersionSet`] — the version-range algebra, closed under intersection.
//!
//! ## Why this is not `ConstraintSpec`
//!
//! [`gen_types::ConstraintSpec`] is a **syntax**: one variant per spelling a
//! package manager surfaces (`^1.2.3`, `~1.2.3`, `>=1.2.3`, `>1.2.3`, …). It
//! answers *does this version match* and nothing else, which is all a manifest
//! reader needs.
//!
//! A solver needs a different question answered: *what versions satisfy ALL of
//! these at once, and is that set empty?* That requires a representation closed
//! under intersection, and `ConstraintSpec` is not — there is no variant for
//! `>1.0.0 ∩ <=2.0.0`, and no variant for "nothing". Adding one would push
//! solver arithmetic into the syntax every adapter parses into, which is the
//! wrong place for it.
//!
//! So the syntax is **lowered** here, once, into a union of disjoint intervals
//! over the total order [`gen_types::Version`] already defines. Intersection,
//! union and emptiness are then exact rather than approximated.
//!
//! ## This algebra is NOT a lattice meet/join over postures
//!
//! Stated because the fleet has a standing trap here: a posture lattice's
//! meet/join and a version-set intersection look alike and are not the same
//! operation. A posture meet picks *one* element of an ordered lattice; a
//! version-set intersection computes a *subset* of a totally-ordered domain and
//! can be empty, which is the whole point — an empty intersection is the fact a
//! conflict report is built from. Nothing in this module knows what a posture
//! is, and nothing in it should learn.
//!
//! ## Bounds are explicit, and that is load-bearing
//!
//! A half-open `[lo, hi)` representation needs a successor function to encode
//! `<=v` (as `< next(v)`). With pre-releases in the domain there is no
//! successor: `1.2.3-rc.0.1` sits strictly between `1.2.3-rc.0` and
//! `1.2.3-rc.1`, and you can always insert another. [`Bound`] therefore carries
//! its inclusivity, and `<=v` is representable exactly instead of nearly.

use core::fmt;
use gen_types::{Combinator, CompoundConstraint, ConstraintSpec, Version, VersionConstraint};
use std::cmp::Ordering;

/// One end of an interval.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Bound {
    /// No bound on this side.
    Unbounded,
    /// The version itself is in the set.
    Included(Version),
    /// The version itself is not in the set.
    Excluded(Version),
}

impl Bound {
    /// The version this bound names, if it names one.
    #[must_use]
    pub const fn version(&self) -> Option<&Version> {
        match self {
            Self::Unbounded => None,
            Self::Included(v) | Self::Excluded(v) => Some(v),
        }
    }

    /// Order two bounds **used as lower bounds**. `Unbounded` is smallest, and
    /// at an equal version `Excluded` is the *higher* floor because it admits
    /// strictly less.
    fn cmp_as_lower(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Unbounded, Self::Unbounded) => Ordering::Equal,
            (Self::Unbounded, _) => Ordering::Less,
            (_, Self::Unbounded) => Ordering::Greater,
            _ => {
                let (a, b) = (
                    self.version().expect("bounded"),
                    other.version().expect("bounded"),
                );
                a.cmp(b).then_with(|| {
                    let strict = |x: &Self| u8::from(matches!(x, Self::Excluded(_)));
                    strict(self).cmp(&strict(other))
                })
            }
        }
    }

    /// Order two bounds **used as upper bounds**. `Unbounded` is largest, and
    /// at an equal version `Excluded` is the *lower* ceiling.
    fn cmp_as_upper(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Unbounded, Self::Unbounded) => Ordering::Equal,
            (Self::Unbounded, _) => Ordering::Greater,
            (_, Self::Unbounded) => Ordering::Less,
            _ => {
                let (a, b) = (
                    self.version().expect("bounded"),
                    other.version().expect("bounded"),
                );
                a.cmp(b).then_with(|| {
                    let strict = |x: &Self| u8::from(matches!(x, Self::Included(_)));
                    strict(self).cmp(&strict(other))
                })
            }
        }
    }
}

/// A single contiguous run of versions. Always non-empty — the constructor is
/// the only way to build one and it returns `None` for an empty span, so an
/// empty interval has no inhabitant to reason about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Interval {
    lo: Bound,
    hi: Bound,
}

impl Interval {
    /// Build an interval, or `None` if the bounds admit nothing.
    #[must_use]
    pub fn new(lo: Bound, hi: Bound) -> Option<Self> {
        let inhabited = match (lo.version(), hi.version()) {
            (Some(l), Some(h)) => match l.cmp(h) {
                Ordering::Less => true,
                // A single point survives only when both ends include it.
                Ordering::Equal => {
                    matches!(lo, Bound::Included(_)) && matches!(hi, Bound::Included(_))
                }
                Ordering::Greater => false,
            },
            _ => true,
        };
        inhabited.then_some(Self { lo, hi })
    }

    #[must_use]
    pub const fn lower(&self) -> &Bound {
        &self.lo
    }

    #[must_use]
    pub const fn upper(&self) -> &Bound {
        &self.hi
    }

    #[must_use]
    pub fn contains(&self, v: &Version) -> bool {
        let above_floor = match &self.lo {
            Bound::Unbounded => true,
            Bound::Included(l) => v >= l,
            Bound::Excluded(l) => v > l,
        };
        let below_ceiling = match &self.hi {
            Bound::Unbounded => true,
            Bound::Included(h) => v <= h,
            Bound::Excluded(h) => v < h,
        };
        above_floor && below_ceiling
    }

    fn intersect(&self, other: &Self) -> Option<Self> {
        let lo = if self.lo.cmp_as_lower(&other.lo) == Ordering::Greater {
            self.lo.clone()
        } else {
            other.lo.clone()
        };
        let hi = if self.hi.cmp_as_upper(&other.hi) == Ordering::Less {
            self.hi.clone()
        } else {
            other.hi.clone()
        };
        Self::new(lo, hi)
    }

    /// Can these two be replaced by one interval covering exactly their union?
    ///
    /// True when they overlap **or** merely touch: `[1,2)` and `[2,3]` leave no
    /// gap, so keeping them apart would make two spellings of one set. `[1,2)`
    /// and `(2,3]` do leave a gap — `2` itself — and stay separate.
    fn joinable_with(&self, other: &Self) -> bool {
        // Assumes `self` sorts at or before `other` by lower bound.
        match (self.hi.version(), other.lo.version()) {
            (None, _) | (_, None) => true,
            (Some(h), Some(l)) => match h.cmp(l) {
                Ordering::Greater => true,
                Ordering::Equal => {
                    matches!(self.hi, Bound::Included(_)) || matches!(other.lo, Bound::Included(_))
                }
                Ordering::Less => false,
            },
        }
    }

    fn join(self, other: Self) -> Self {
        let hi = if self.hi.cmp_as_upper(&other.hi) == Ordering::Greater {
            self.hi
        } else {
            other.hi
        };
        Self { lo: self.lo, hi }
    }
}

impl fmt::Display for Interval {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (&self.lo, &self.hi) {
            (Bound::Unbounded, Bound::Unbounded) => f.write_str("*"),
            (Bound::Included(l), Bound::Included(h)) if l == h => write!(f, "={l}"),
            _ => {
                match &self.lo {
                    Bound::Unbounded => Ok(()),
                    Bound::Included(l) => write!(f, ">={l}"),
                    Bound::Excluded(l) => write!(f, ">{l}"),
                }?;
                if !matches!(self.lo, Bound::Unbounded) && !matches!(self.hi, Bound::Unbounded) {
                    f.write_str(", ")?;
                }
                match &self.hi {
                    Bound::Unbounded => Ok(()),
                    Bound::Included(h) => write!(f, "<={h}"),
                    Bound::Excluded(h) => write!(f, "<{h}"),
                }
            }
        }
    }
}

/// A set of acceptable versions: a union of disjoint, non-touching intervals
/// held in ascending order.
///
/// That canonical form is what makes `==` mean *the same versions* rather than
/// *the same spelling*, so a caller never has to ask a separate `same_set_as`
/// question. `ConstraintSpec` deliberately keeps the spelling (`Caret` and the
/// equivalent `Range` are distinct there, for round-tripping); this type
/// deliberately loses it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VersionSet {
    /// Invariant: disjoint, non-adjacent, ascending by lower bound. Empty means
    /// the set is empty.
    runs: Vec<Interval>,
}

impl VersionSet {
    /// The set admitting nothing.
    #[must_use]
    pub const fn empty() -> Self {
        Self { runs: Vec::new() }
    }

    /// The set admitting every version — the intersection identity.
    #[must_use]
    pub fn any() -> Self {
        Self {
            runs: vec![Interval {
                lo: Bound::Unbounded,
                hi: Bound::Unbounded,
            }],
        }
    }

    /// Exactly one version.
    #[must_use]
    pub fn exactly(v: Version) -> Self {
        // `Interval::new` is the only constructor and it is fallible, so this
        // goes through `into_iter` rather than an `expect`: a closed point is
        // always inhabited, but asserting that here would be a second place
        // that has to stay true if `new` ever tightens.
        Self::of_intervals(
            Interval::new(Bound::Included(v.clone()), Bound::Included(v))
                .into_iter()
                .collect(),
        )
    }

    /// Normalize a bag of intervals into the canonical form.
    fn of_intervals(mut runs: Vec<Interval>) -> Self {
        runs.sort_by(|a, b| {
            a.lo.cmp_as_lower(&b.lo)
                .then_with(|| a.hi.cmp_as_upper(&b.hi))
        });
        let mut merged: Vec<Interval> = Vec::with_capacity(runs.len());
        for run in runs {
            match merged.pop() {
                None => merged.push(run),
                Some(prev) => {
                    if prev.joinable_with(&run) {
                        merged.push(prev.join(run));
                    } else {
                        merged.push(prev);
                        merged.push(run);
                    }
                }
            }
        }
        Self { runs: merged }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.runs.is_empty()
    }

    #[must_use]
    pub fn contains(&self, v: &Version) -> bool {
        self.runs.iter().any(|r| r.contains(v))
    }

    /// The contiguous runs, ascending. Exposed so a diagnostic can describe the
    /// shape of a set without re-deriving it from the constraints.
    #[must_use]
    pub fn intervals(&self) -> &[Interval] {
        &self.runs
    }

    /// Versions in both. **Empty is a real answer**, and it is the fact a
    /// conflict report is built from — not an error and not a sentinel range
    /// that happens to admit nothing.
    #[must_use]
    pub fn intersect(&self, other: &Self) -> Self {
        let mut out = Vec::new();
        for a in &self.runs {
            for b in &other.runs {
                if let Some(hit) = a.intersect(b) {
                    out.push(hit);
                }
            }
        }
        Self::of_intervals(out)
    }

    /// Versions in either. Needed because [`Combinator::Or`] is already part of
    /// gen's constraint language — a solver that could only conjoin would
    /// silently mis-read `^1 || ^2`.
    #[must_use]
    pub fn union(&self, other: &Self) -> Self {
        let mut out = self.runs.clone();
        out.extend(other.runs.iter().cloned());
        Self::of_intervals(out)
    }

    /// Lower one [`ConstraintSpec`] spelling into the algebra.
    #[must_use]
    pub fn of_spec(spec: &ConstraintSpec) -> Self {
        let one = |lo: Bound, hi: Bound| Self::of_intervals(Interval::new(lo, hi).into_iter().collect());
        match spec {
            ConstraintSpec::Exact(v) => Self::exactly(v.clone()),
            ConstraintSpec::Range {
                lower_inclusive,
                upper_exclusive,
            } => one(
                Bound::Included(lower_inclusive.clone()),
                Bound::Excluded(upper_exclusive.clone()),
            ),
            ConstraintSpec::Tilde(base) => one(
                Bound::Included(base.clone()),
                Bound::Excluded(Version::new(base.major, base.minor + 1, 0)),
            ),
            ConstraintSpec::Caret(base) => one(
                Bound::Included(base.clone()),
                Bound::Excluded(caret_ceiling(base)),
            ),
            ConstraintSpec::GreaterEqual(v) => one(Bound::Included(v.clone()), Bound::Unbounded),
            ConstraintSpec::Greater(v) => one(Bound::Excluded(v.clone()), Bound::Unbounded),
            ConstraintSpec::LessEqual(v) => one(Bound::Unbounded, Bound::Included(v.clone())),
            ConstraintSpec::Less(v) => one(Bound::Unbounded, Bound::Excluded(v.clone())),
            ConstraintSpec::Any => Self::any(),
        }
    }

    /// Lower a [`VersionConstraint`]. The retained native syntax is ignored —
    /// it is a diagnostic, never an input to resolution.
    #[must_use]
    pub fn of_constraint(c: &VersionConstraint) -> Self {
        Self::of_spec(&c.spec)
    }

    /// Lower a [`CompoundConstraint`]: `Or` folds by union, `And` by
    /// intersection. An `And` over no atoms is [`VersionSet::any`] and an `Or`
    /// over no atoms is [`VersionSet::empty`] — the identity of each fold, so
    /// an adapter that emits an empty compound gets the mathematically correct
    /// answer rather than a special case.
    #[must_use]
    pub fn of_compound(c: &CompoundConstraint) -> Self {
        match c.combinator {
            Combinator::Or => c
                .atoms
                .iter()
                .fold(Self::empty(), |acc, a| acc.union(&Self::of_spec(a))),
            Combinator::And => c
                .atoms
                .iter()
                .fold(Self::any(), |acc, a| acc.intersect(&Self::of_spec(a))),
        }
    }
}

/// The exclusive ceiling of `^v`.
///
/// Cargo's rule, matching [`ConstraintSpec::Caret`]'s own implementation in
/// gen-types: below `1.0` the *minor* is the breaking component, and below
/// `0.1` the patch is.
fn caret_ceiling(v: &Version) -> Version {
    if v.major > 0 {
        Version::new(v.major + 1, 0, 0)
    } else if v.minor > 0 {
        Version::new(0, v.minor + 1, 0)
    } else {
        Version::new(0, 0, v.patch + 1)
    }
}

impl fmt::Display for VersionSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.runs.is_empty() {
            return f.write_str("<nothing>");
        }
        for (i, run) in self.runs.iter().enumerate() {
            if i > 0 {
                f.write_str(" || ")?;
            }
            write!(f, "{run}")?;
        }
        Ok(())
    }
}

/// Highest first — the order a solver tries candidates in, deduplicated.
#[must_use]
pub fn newest_first(mut versions: Vec<Version>) -> Vec<Version> {
    versions.sort_by(|a, b| b.cmp(a));
    versions.dedup();
    versions
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(a: u64, b: u64, c: u64) -> Version {
        Version::new(a, b, c)
    }

    #[test]
    fn an_empty_set_admits_nothing_and_says_so() {
        let e = VersionSet::empty();
        assert!(e.is_empty());
        assert!(!e.contains(&v(1, 0, 0)));
        assert_eq!(e.to_string(), "<nothing>");
    }

    #[test]
    fn any_is_the_intersection_identity() {
        for spec in [
            ConstraintSpec::Caret(v(1, 2, 3)),
            ConstraintSpec::Greater(v(1, 0, 0)),
            ConstraintSpec::LessEqual(v(2, 0, 0)),
            ConstraintSpec::Exact(v(1, 5, 0)),
        ] {
            let s = VersionSet::of_spec(&spec);
            assert_eq!(s.intersect(&VersionSet::any()), s, "identity for {spec:?}");
        }
    }

    /// **The shape `ConstraintSpec` cannot express.** A strict lower bound met
    /// with a closed upper bound has no variant in the syntax; it has an exact
    /// representation here.
    #[test]
    fn a_strict_lower_bound_survives_intersection_exactly() {
        let gt = VersionSet::of_spec(&ConstraintSpec::Greater(v(1, 0, 0)));
        let le = VersionSet::of_spec(&ConstraintSpec::LessEqual(v(2, 0, 0)));
        let both = gt.intersect(&le);
        assert!(!both.contains(&v(1, 0, 0)), "the floor is excluded");
        assert!(both.contains(&v(1, 0, 1)));
        assert!(both.contains(&v(2, 0, 0)), "the ceiling is included");
        assert!(!both.contains(&v(2, 0, 1)));
        assert_eq!(both.to_string(), ">1.0.0, <=2.0.0");
    }

    #[test]
    fn incompatible_constraints_intersect_to_empty() {
        let a = VersionSet::of_spec(&ConstraintSpec::Caret(v(1, 0, 0)));
        let b = VersionSet::of_spec(&ConstraintSpec::Caret(v(2, 0, 0)));
        assert!(a.intersect(&b).is_empty());
    }

    /// **Two spellings of one set are one value.** `^1.0.0` and
    /// `>=1.0.0, <2.0.0` are distinct `ConstraintSpec`s on purpose; here they
    /// must be equal, or every conflict report would depend on which adapter
    /// happened to parse the manifest.
    #[test]
    fn equality_is_by_set_not_by_spelling() {
        let caret = VersionSet::of_spec(&ConstraintSpec::Caret(v(1, 0, 0)));
        let range = VersionSet::of_spec(&ConstraintSpec::Range {
            lower_inclusive: v(1, 0, 0),
            upper_exclusive: v(2, 0, 0),
        });
        assert_eq!(caret, range);
        assert_ne!(caret, VersionSet::of_spec(&ConstraintSpec::Caret(v(2, 0, 0))));
    }

    /// A disjunction stays two runs when a gap separates them, and collapses to
    /// one when they touch. Both directions, or the canonical form is only
    /// half-tested.
    #[test]
    fn a_union_merges_touching_runs_and_keeps_separated_ones() {
        let touching = VersionSet::of_compound(&CompoundConstraint {
            combinator: Combinator::Or,
            atoms: vec![
                ConstraintSpec::Range {
                    lower_inclusive: v(1, 0, 0),
                    upper_exclusive: v(2, 0, 0),
                },
                ConstraintSpec::Range {
                    lower_inclusive: v(2, 0, 0),
                    upper_exclusive: v(3, 0, 0),
                },
            ],
        });
        assert_eq!(touching.intervals().len(), 1, "{touching}");
        assert!(touching.contains(&v(2, 0, 0)));

        let separated = VersionSet::of_compound(&CompoundConstraint {
            combinator: Combinator::Or,
            atoms: vec![ConstraintSpec::Caret(v(1, 0, 0)), ConstraintSpec::Caret(v(3, 0, 0))],
        });
        assert_eq!(separated.intervals().len(), 2, "{separated}");
        assert!(!separated.contains(&v(2, 5, 0)));
        assert_eq!(separated.to_string(), ">=1.0.0, <2.0.0 || >=3.0.0, <4.0.0");
    }

    /// Two runs that both *exclude* the version between them must NOT merge —
    /// the gap is real even though it is a single point.
    #[test]
    fn a_single_point_gap_is_a_real_gap() {
        let s = VersionSet::of_spec(&ConstraintSpec::Less(v(2, 0, 0)))
            .union(&VersionSet::of_spec(&ConstraintSpec::Greater(v(2, 0, 0))));
        assert_eq!(s.intervals().len(), 2, "{s}");
        assert!(!s.contains(&v(2, 0, 0)));
        assert!(s.contains(&v(1, 9, 9)));
        assert!(s.contains(&v(2, 0, 1)));
    }

    /// Intersecting a disjunction distributes: `(^1 || ^3) ∩ >=2` keeps only
    /// the `^3` run.
    #[test]
    fn intersection_distributes_over_a_disjunction() {
        let either = VersionSet::of_compound(&CompoundConstraint {
            combinator: Combinator::Or,
            atoms: vec![ConstraintSpec::Caret(v(1, 0, 0)), ConstraintSpec::Caret(v(3, 0, 0))],
        });
        let narrowed = either.intersect(&VersionSet::of_spec(&ConstraintSpec::GreaterEqual(v(2, 0, 0))));
        assert_eq!(narrowed, VersionSet::of_spec(&ConstraintSpec::Caret(v(3, 0, 0))));
    }

    /// An empty fold returns the identity of its combinator, not a special
    /// case: `And` over nothing admits everything, `Or` over nothing admits
    /// nothing.
    #[test]
    fn an_empty_compound_folds_to_its_identity() {
        let and = VersionSet::of_compound(&CompoundConstraint {
            combinator: Combinator::And,
            atoms: vec![],
        });
        let or = VersionSet::of_compound(&CompoundConstraint {
            combinator: Combinator::Or,
            atoms: vec![],
        });
        assert_eq!(and, VersionSet::any());
        assert!(or.is_empty());
    }

    /// Pre-releases sort below their release, so `<1.2.3` admits `1.2.3-rc.1`.
    /// This is SemVer's rule and gen-types already implements the ordering;
    /// the set algebra must not quietly disagree with it.
    #[test]
    fn pre_releases_live_below_their_release_in_the_order() {
        let rc = Version::parse("1.2.3-rc.1").expect("parses");
        let lt = VersionSet::of_spec(&ConstraintSpec::Less(v(1, 2, 3)));
        assert!(lt.contains(&rc));
        assert!(!lt.contains(&v(1, 2, 3)));
    }

    #[test]
    fn newest_first_sorts_descending_and_dedups() {
        let out = newest_first(vec![v(1, 0, 0), v(2, 0, 0), v(1, 0, 0), v(1, 5, 0)]);
        assert_eq!(out, vec![v(2, 0, 0), v(1, 5, 0), v(1, 0, 0)]);
    }
}
