//! Typed [`VersionConstraint`] — what versions a dependency accepts.
//!
//! Adapters parse their native constraint syntax (cargo `^1.2.3`,
//! npm `>=1.2.3 <2.0.0`, Bundler `~> 1.2.3`, pip `>=1.2,<2.0`,
//! Composer `^1.2 || ^2.0`, …) into this canonical typed shape.
//! The resolver in `gen-engine` reads this once + does its
//! version-selection math against it; per-adapter constraint logic
//! lives only at the parse boundary.

use crate::Version;
use serde::{Deserialize, Serialize};

/// One typed variant per constraint kind every package manager
/// surfaces. Adapter is responsible for converting `~> 1.2.3` etc.
/// into the matching shape.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ConstraintSpec {
    /// Match exactly one version: `=1.2.3`.
    Exact(Version),
    /// Inclusive lower bound, exclusive upper bound: `>=1.2.3,<2.0.0`.
    Range { lower_inclusive: Version, upper_exclusive: Version },
    /// `~1.2.3` — patch-level: `>=1.2.3,<1.3.0`. Caller-friendly
    /// alias for the common "approximately equal to" shape.
    Tilde(Version),
    /// `^1.2.3` — minor-level (default Cargo behavior): `>=1.2.3,<2.0.0`.
    /// Identical to `Range { 1.2.3, 2.0.0 }` — kept as a separate
    /// variant for diagnostics + round-tripping.
    Caret(Version),
    /// `>=1.2.3` — open upper bound. Common in npm + pip.
    GreaterEqual(Version),
    /// `>1.2.3` — strict lower bound.
    Greater(Version),
    /// `<=1.2.3` — closed upper bound.
    LessEqual(Version),
    /// `<1.2.3` — strict upper bound.
    Less(Version),
    /// `*` / `any` — matches every version. Last-resort.
    Any,
}

/// Compound constraint container — disjunction / conjunction of
/// atomic `ConstraintSpec`s. Kept separate from `ConstraintSpec` to
/// avoid serde recursion-overflow on the derive macros (Vec<Self>
/// inside a derived Serialize/Deserialize enum triggers a
/// monomorphization storm). Most adapters never need this — the
/// `Range` + `Caret` + `Tilde` variants cover ≥95% of real-world
/// constraints.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompoundConstraint {
    pub combinator: Combinator,
    pub atoms: Vec<ConstraintSpec>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Combinator {
    /// Disjunction: `^1.2.3 || ^2.0.0`. Composer + npm support this.
    Or,
    /// Conjunction: `>=1.2,<2.0` (the and-of-two-constraints shape
    /// some adapters surface separately from `Range`).
    And,
}

impl CompoundConstraint {
    #[must_use]
    pub fn matches(&self, v: &Version) -> bool {
        match self.combinator {
            Combinator::Or => self.atoms.iter().any(|a| match_constraint(a, v)),
            Combinator::And => self.atoms.iter().all(|a| match_constraint(a, v)),
        }
    }
}

/// Wrapper carrying both the typed [`ConstraintSpec`] and the
/// original native syntax — handy for diagnostics + round-tripping
/// back to the source manifest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionConstraint {
    pub spec: ConstraintSpec,
    /// Original adapter-native syntax (e.g. `~> 1.2.3` for Bundler).
    /// Optional — adapters that lose the original syntax during
    /// parsing leave this `None`.
    #[serde(default)]
    pub native_syntax: Option<String>,
}

impl VersionConstraint {
    /// Convenience: build from a typed spec without retaining the
    /// native syntax. Useful in tests + adapter-independent code.
    #[must_use]
    pub const fn from_spec(spec: ConstraintSpec) -> Self {
        Self {
            spec,
            native_syntax: None,
        }
    }

    /// Does this constraint accept the supplied version? Pure
    /// function over the typed spec — adapter is never consulted.
    #[must_use]
    pub fn matches(&self, v: &Version) -> bool {
        match_constraint(&self.spec, v)
    }
}

fn match_constraint(c: &ConstraintSpec, v: &Version) -> bool {
    match c {
        ConstraintSpec::Exact(target) => v == target,
        ConstraintSpec::Range { lower_inclusive, upper_exclusive } => {
            v >= lower_inclusive && v < upper_exclusive
        }
        ConstraintSpec::Tilde(base) => {
            // ~1.2.3 → >=1.2.3, <1.3.0
            v >= base
                && v < &Version::new(base.major, base.minor + 1, 0)
        }
        ConstraintSpec::Caret(base) => {
            // ^1.2.3 → >=1.2.3, <2.0.0 (when major > 0)
            // ^0.2.3 → >=0.2.3, <0.3.0 (cargo edge case for 0.x)
            // ^0.0.3 → >=0.0.3, <0.0.4 (cargo edge case for 0.0.x)
            if base.major > 0 {
                v >= base && v < &Version::new(base.major + 1, 0, 0)
            } else if base.minor > 0 {
                v >= base && v < &Version::new(0, base.minor + 1, 0)
            } else {
                v >= base && v < &Version::new(0, 0, base.patch + 1)
            }
        }
        ConstraintSpec::GreaterEqual(target) => v >= target,
        ConstraintSpec::Greater(target) => v > target,
        ConstraintSpec::LessEqual(target) => v <= target,
        ConstraintSpec::Less(target) => v < target,
        ConstraintSpec::Any => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_matches_only_exact_version() {
        let c = VersionConstraint::from_spec(ConstraintSpec::Exact(Version::new(1, 2, 3)));
        assert!(c.matches(&Version::new(1, 2, 3)));
        assert!(!c.matches(&Version::new(1, 2, 4)));
    }

    #[test]
    fn caret_major_above_zero() {
        let c = VersionConstraint::from_spec(ConstraintSpec::Caret(Version::new(1, 2, 3)));
        assert!(c.matches(&Version::new(1, 2, 3)));
        assert!(c.matches(&Version::new(1, 99, 0)));
        assert!(!c.matches(&Version::new(2, 0, 0)));
        assert!(!c.matches(&Version::new(1, 2, 2)));
    }

    #[test]
    fn caret_major_zero_minor_above_zero() {
        // ^0.2.3 → >=0.2.3, <0.3.0
        let c = VersionConstraint::from_spec(ConstraintSpec::Caret(Version::new(0, 2, 3)));
        assert!(c.matches(&Version::new(0, 2, 3)));
        assert!(c.matches(&Version::new(0, 2, 99)));
        assert!(!c.matches(&Version::new(0, 3, 0)));
    }

    #[test]
    fn caret_major_and_minor_zero() {
        // ^0.0.3 → >=0.0.3, <0.0.4
        let c = VersionConstraint::from_spec(ConstraintSpec::Caret(Version::new(0, 0, 3)));
        assert!(c.matches(&Version::new(0, 0, 3)));
        assert!(!c.matches(&Version::new(0, 0, 4)));
    }

    #[test]
    fn tilde_only_allows_patch_changes() {
        // ~1.2.3 → >=1.2.3, <1.3.0
        let c = VersionConstraint::from_spec(ConstraintSpec::Tilde(Version::new(1, 2, 3)));
        assert!(c.matches(&Version::new(1, 2, 3)));
        assert!(c.matches(&Version::new(1, 2, 99)));
        assert!(!c.matches(&Version::new(1, 3, 0)));
    }

    #[test]
    fn range_inclusive_lower_exclusive_upper() {
        let c = VersionConstraint::from_spec(ConstraintSpec::Range {
            lower_inclusive: Version::new(1, 2, 3),
            upper_exclusive: Version::new(2, 0, 0),
        });
        assert!(c.matches(&Version::new(1, 2, 3)));
        assert!(c.matches(&Version::new(1, 99, 99)));
        assert!(!c.matches(&Version::new(2, 0, 0)));
        assert!(!c.matches(&Version::new(1, 2, 2)));
    }

    #[test]
    fn open_bounds() {
        let ge = VersionConstraint::from_spec(ConstraintSpec::GreaterEqual(Version::new(1, 0, 0)));
        let gt = VersionConstraint::from_spec(ConstraintSpec::Greater(Version::new(1, 0, 0)));
        let le = VersionConstraint::from_spec(ConstraintSpec::LessEqual(Version::new(1, 0, 0)));
        let lt = VersionConstraint::from_spec(ConstraintSpec::Less(Version::new(1, 0, 0)));
        assert!(ge.matches(&Version::new(1, 0, 0)));
        assert!(!gt.matches(&Version::new(1, 0, 0)));
        assert!(le.matches(&Version::new(1, 0, 0)));
        assert!(!lt.matches(&Version::new(1, 0, 0)));
    }

    #[test]
    fn any_matches_everything() {
        let c = VersionConstraint::from_spec(ConstraintSpec::Any);
        assert!(c.matches(&Version::new(0, 0, 0)));
        assert!(c.matches(&Version::new(999, 999, 999)));
    }

    #[test]
    fn disjunction_via_compound() {
        let c = CompoundConstraint {
            combinator: Combinator::Or,
            atoms: vec![
                ConstraintSpec::Caret(Version::new(1, 0, 0)),
                ConstraintSpec::Caret(Version::new(2, 0, 0)),
            ],
        };
        assert!(c.matches(&Version::new(1, 5, 0)));
        assert!(c.matches(&Version::new(2, 5, 0)));
        assert!(!c.matches(&Version::new(3, 0, 0)));
    }

    #[test]
    fn conjunction_via_compound() {
        let c = CompoundConstraint {
            combinator: Combinator::And,
            atoms: vec![
                ConstraintSpec::GreaterEqual(Version::new(1, 2, 0)),
                ConstraintSpec::Less(Version::new(2, 0, 0)),
            ],
        };
        assert!(c.matches(&Version::new(1, 5, 0)));
        assert!(!c.matches(&Version::new(1, 1, 0)));
        assert!(!c.matches(&Version::new(2, 0, 0)));
    }

    #[test]
    fn round_trip_through_serde() {
        let c = VersionConstraint {
            spec: ConstraintSpec::Caret(Version::new(1, 2, 3)),
            native_syntax: Some("^1.2.3".to_string()),
        };
        let j = serde_json::to_string(&c).unwrap();
        let parsed: VersionConstraint = serde_json::from_str(&j).unwrap();
        assert_eq!(c, parsed);
    }
}
