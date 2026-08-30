//! [`PackageUniverse`] — where candidate versions and their edges come from.
//!
//! A trait, so resolution is testable without a network, a registry server or
//! a filesystem — the three things that make a solver's tests slow and flaky
//! everywhere else. It is also the seam a real registry client lands behind.
//!
//! ## Why not call it `Registry`
//!
//! [`gen_types::Registry`] already exists and means something else: *which
//! upstream hosts this package* (crates.io, npm, an OCI ref). Reusing the word
//! for "the thing that answers version queries" would make two unrelated
//! concepts share a name in one dependency tree.

use crate::set::newest_first;
use gen_types::{Dependency, DependencyKind, Version};
use std::collections::BTreeMap;

/// Which dependency edges resolution follows.
///
/// Not a bag of booleans: an enum, so the choice is one exhaustively-matched
/// value and adding a policy is a compile error at every decision site rather
/// than a silently-defaulted field.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Follow {
    /// Every declared edge, whatever its kind.
    ///
    /// The default **on purpose**. Dropping edges narrows the problem, and a
    /// solver that silently narrows can report a satisfiable graph for a
    /// manifest that is not — the failure mode you find at install time, not
    /// resolve time. Narrowing is the caller's decision to make out loud.
    #[default]
    All,
    /// Only edges [`DependencyKind::is_runtime`] admits — dev and build edges
    /// are skipped. Reuses gen-types' own predicate rather than re-deriving
    /// the classification.
    Runtime,
}

impl Follow {
    #[must_use]
    pub const fn admits(self, kind: DependencyKind) -> bool {
        match self {
            Self::All => true,
            Self::Runtime => kind.is_runtime(),
        }
    }
}

/// The candidate space resolution searches.
pub trait PackageUniverse {
    /// Every published version of `name`, in any order — the solver sorts.
    fn versions(&self, name: &str) -> Vec<Version>;

    /// What `name@version` depends on. `None` if that exact version does not
    /// exist, which is distinct from `Some(vec![])` — "no such version" and
    /// "a version with no dependencies" lead to different diagnostics.
    fn dependencies(&self, name: &str, version: &Version) -> Option<Vec<Dependency>>;
}

/// An in-memory universe, for tests and for callers that already hold the whole
/// graph.
#[derive(Clone, Debug, Default)]
pub struct MapUniverse {
    /// Keyed by name then version so `versions()` is a single map lookup
    /// instead of a scan, and so iteration order is deterministic.
    entries: BTreeMap<String, BTreeMap<Version, Vec<Dependency>>>,
}

impl MapUniverse {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Publish `name@version` with the edges it declares.
    pub fn add(&mut self, name: &str, version: Version, deps: Vec<Dependency>) -> &mut Self {
        self.entries
            .entry(name.to_string())
            .or_default()
            .insert(version, deps);
        self
    }

    /// Publish a version with no edges.
    pub fn add_leaf(&mut self, name: &str, version: Version) -> &mut Self {
        self.add(name, version, Vec::new())
    }
}

impl PackageUniverse for MapUniverse {
    fn versions(&self, name: &str) -> Vec<Version> {
        self.entries
            .get(name)
            .map(|vs| newest_first(vs.keys().cloned().collect()))
            .unwrap_or_default()
    }

    fn dependencies(&self, name: &str, version: &Version) -> Option<Vec<Dependency>> {
        self.entries.get(name)?.get(version).cloned()
    }
}

/// Build a plain runtime [`Dependency`] on `name` with the given constraint.
///
/// [`Dependency`] has eight fields and a solver test cares about two of them.
/// Without this helper every fixture repeats six defaults, which is how a
/// fixture ends up disagreeing with the next one about what "a normal edge"
/// means.
#[must_use]
pub fn edge(name: &str, spec: gen_types::ConstraintSpec) -> Dependency {
    edge_of_kind(name, spec, DependencyKind::Direct)
}

/// [`edge`], with the kind chosen explicitly.
#[must_use]
pub fn edge_of_kind(
    name: &str,
    spec: gen_types::ConstraintSpec,
    kind: DependencyKind,
) -> Dependency {
    Dependency {
        name: name.to_string(),
        constraint: gen_types::VersionConstraint::from_spec(spec),
        kind,
        features_enabled: Vec::new(),
        default_features: true,
        target_predicate: None,
        source_override: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gen_types::ConstraintSpec;

    #[test]
    fn versions_come_back_newest_first() {
        let mut u = MapUniverse::new();
        u.add_leaf("a", Version::new(1, 0, 0))
            .add_leaf("a", Version::new(2, 0, 0))
            .add_leaf("a", Version::new(1, 5, 0));
        assert_eq!(
            u.versions("a"),
            vec![
                Version::new(2, 0, 0),
                Version::new(1, 5, 0),
                Version::new(1, 0, 0)
            ]
        );
    }

    /// "No such version" and "a version with no edges" are different answers,
    /// and a caller that cannot tell them apart cannot write the right message.
    #[test]
    fn an_absent_version_is_none_not_an_empty_edge_list() {
        let mut u = MapUniverse::new();
        u.add_leaf("a", Version::new(1, 0, 0));
        assert_eq!(
            u.dependencies("a", &Version::new(1, 0, 0)),
            Some(Vec::new())
        );
        assert_eq!(u.dependencies("a", &Version::new(9, 0, 0)), None);
        assert_eq!(u.dependencies("ghost", &Version::new(1, 0, 0)), None);
        assert!(u.versions("ghost").is_empty());
    }

    #[test]
    fn follow_runtime_defers_to_gen_types_own_predicate() {
        for kind in [
            DependencyKind::Direct,
            DependencyKind::Build,
            DependencyKind::Dev,
            DependencyKind::Optional,
            DependencyKind::Peer,
            DependencyKind::Replaces,
        ] {
            assert!(Follow::All.admits(kind), "All admits {kind:?}");
            assert_eq!(
                Follow::Runtime.admits(kind),
                kind.is_runtime(),
                "Runtime must not re-derive the classification for {kind:?}"
            );
        }
        assert_eq!(Follow::default(), Follow::All);
    }

    #[test]
    fn the_edge_helper_builds_a_plain_direct_dependency() {
        let d = edge("serde", ConstraintSpec::Caret(Version::new(1, 0, 0)));
        assert_eq!(d.name, "serde");
        assert_eq!(d.kind, DependencyKind::Direct);
        assert!(d.default_features);
        assert!(d.target_predicate.is_none());
    }
}
