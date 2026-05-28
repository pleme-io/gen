//! Cross-ecosystem trait surface.
//!
//! Three traits every package-manager adapter implements to fit the
//! pleme-io substrate's universal intake pattern. The contract is
//! documented in `theory/ECOSYSTEM-INTAKE.md`; this module IS the
//! Rust binding of that contract.
//!
//! Stack:
//! - [`Spec`] — the typed shape of an adapter's emitted build spec
//! - [`QuirkRegistry`] — the typed shape of known upstream-bug quirks
//! - [`Invariants`] — the typed shape of well-formedness checks
//!
//! Every concrete adapter (gen-cargo today; gen-npm + gen-bundler +
//! gen-pip + gen-helm + gen-gomod as they ship) implements all three
//! on its own typed shapes. Substrate / cse-lint / gen confirm
//! consume these traits without caring which ecosystem produced
//! them.

use serde::{de::DeserializeOwned, Serialize};

/// The typed shape an adapter's build spec exposes to substrate +
/// tooling consumers. Every adapter's `BuildSpec` struct implements
/// this; the trait method surface is universal.
///
/// Why the associated types: each ecosystem carries its own
/// `Args` shape (BuildRustCrateArgs for Cargo, NpmInstallArgs for
/// npm, …) and its own `Quirk` enum. The trait doesn't constrain
/// the shapes — substrate dispatches on the serialized JSON. What
/// the trait DOES constrain is the universal accessor surface
/// (schema_version / root_key / member_keys / args_for /
/// quirks_for) so substrate's Nix dispatch is written once.
pub trait Spec: Serialize + DeserializeOwned {
    /// Per-package buildRustCrate-equivalent args.
    type Args: Serialize + DeserializeOwned;
    /// Per-package quirks variant set (typed by the adapter).
    type Quirk: Serialize + DeserializeOwned;

    /// Spec schema version. Substrate's lockfile-builder asserts
    /// this >= a known floor; cse-lint flags stale specs.
    fn schema_version(&self) -> u32;

    /// The workspace's primary buildable package key.
    fn root_key(&self) -> &str;

    /// Every package the workspace tracks (root + members).
    fn member_keys(&self) -> Vec<&str>;

    /// Pre-shaped build args for one package, or None if the package
    /// isn't in the spec.
    fn args_for(&self, key: &str) -> Option<&Self::Args>;

    /// Quirks registered for one package — empty when no quirks
    /// apply, never None.
    fn quirks_for(&self, key: &str) -> &[Self::Quirk];
}

/// The typed shape of an adapter's quirk registry. The registry is
/// the canonical knowledge of "which upstream packages need which
/// build-time workarounds." Each adapter ships its own typed quirk
/// enum + a const REGISTRY table.
///
/// Implementations should be implementable from a TOML side-table
/// (declarative) or a Rust constructor function — both are valid.
/// The `QuirkRegistry` derive macro auto-implements from TOML.
pub trait QuirkRegistry {
    /// The ecosystem's typed quirk variant set.
    type Quirk: Serialize + DeserializeOwned + Clone;

    /// Full registry — every (package_name, quirks) pair the
    /// adapter knows about. Sorted by package name for stable
    /// diffs.
    fn registry() -> Vec<(&'static str, Vec<Self::Quirk>)>;

    /// Lookup quirks for a single package by name. Returns empty
    /// Vec when no quirks registered (never None).
    fn for_package(name: &str) -> Vec<Self::Quirk> {
        for (k, v) in Self::registry() {
            if k == name {
                return v;
            }
        }
        Vec::new()
    }

    /// Every package name with at least one registered quirk. Used
    /// by invariants to detect drift between the registry and the
    /// spec's emission.
    fn registered_names() -> Vec<&'static str> {
        Self::registry().into_iter().map(|(k, _)| k).collect()
    }
}

/// The typed shape of an adapter's invariants pass. Every adapter
/// ships a `check(spec) -> Vec<Violation>` function that verifies
/// the spec is well-formed against its own typed rules.
///
/// `gen confirm` invokes this per-adapter. cse-lint can invoke it
/// fleet-wide.
pub trait Invariants {
    /// The adapter's spec type — pinned to the same `Spec` impl as
    /// the adapter's `build()` returns.
    type Spec: Spec;
    /// The adapter's typed violation enum. Always serializable so
    /// `gen confirm` / cse-lint / IDE tooling can consume the
    /// payload uniformly.
    type Violation: Serialize + DeserializeOwned + Clone;

    /// Run every invariant against the spec. Returns the violation
    /// list (empty when the spec is valid). Pure function, no I/O,
    /// deterministic output.
    fn check(spec: &Self::Spec) -> Vec<Self::Violation>;
}
