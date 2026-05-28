//! gen-cargo's binding to the universal `gen_types::ecosystem` trait
//! surface. Three trait impls bring Cargo into the cross-ecosystem
//! intake pattern (see `theory/ECOSYSTEM-INTAKE.md`).
//!
//! Same data, same code, no copying — these impls delegate to the
//! existing `build_spec` / `quirks` / `invariants` modules. Adding
//! the trait surface is **purely additive**: cargo's own callers
//! still use the concrete types directly; cross-ecosystem callers
//! (cse-lint, future tooling, the universal substrate dispatch)
//! work through the trait.
use gen_types::{Invariants, QuirkRegistry, Spec};

use crate::build_spec::{BuildRustCrateArgs, BuildSpec};
use crate::invariants;
use crate::quirks::CrateQuirk;

impl Spec for BuildSpec {
    type Args = BuildRustCrateArgs;
    type Quirk = CrateQuirk;

    fn schema_version(&self) -> u32 {
        self.version
    }

    fn root_key(&self) -> &str {
        self.root_crate.as_str()
    }

    fn member_keys(&self) -> Vec<&str> {
        self.workspace_members.iter().map(String::as_str).collect()
    }

    fn args_for(&self, key: &str) -> Option<&Self::Args> {
        self.crates.get(key).map(|c| &c.build_rust_crate_args)
    }

    fn quirks_for(&self, key: &str) -> &[Self::Quirk] {
        self.crates
            .get(key)
            .map(|c| c.quirks.as_slice())
            .unwrap_or(&[])
    }
}

/// Marker type used to attach the QuirkRegistry impl. The actual
/// registry lives in `crate::quirks::registry()`; this type is the
/// trait carrier.
pub struct CargoQuirks;

impl QuirkRegistry for CargoQuirks {
    type Quirk = CrateQuirk;

    fn registry() -> Vec<(&'static str, Vec<Self::Quirk>)> {
        crate::quirks::registry()
    }
}

/// Marker type used to attach the Invariants impl. The actual
/// invariant logic lives in `crate::invariants::check`.
pub struct CargoInvariants;

impl Invariants for CargoInvariants {
    type Spec = BuildSpec;
    type Violation = invariants::Violation;

    fn check(spec: &Self::Spec) -> Vec<Self::Violation> {
        invariants::check(spec)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cargo_quirk_registry_implements_trait() {
        let names = <CargoQuirks as QuirkRegistry>::registered_names();
        assert!(!names.is_empty(), "Cargo quirk registry is empty");
        // Sanity: every registered name resolves to non-empty quirks.
        for name in &names {
            let quirks = <CargoQuirks as QuirkRegistry>::for_package(name);
            assert!(!quirks.is_empty(), "{name}: trait lookup returned empty");
        }
    }

    #[test]
    fn unknown_package_returns_empty_quirks_through_trait() {
        let quirks = <CargoQuirks as QuirkRegistry>::for_package("not-a-real-crate-xyz");
        assert!(quirks.is_empty());
    }
}
