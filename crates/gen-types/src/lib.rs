//! gen-types — typed IR for the universal package-manager → build-
//! system engine.
//!
//! Foundation crate. Every adapter (gen-cargo, gen-npm, gen-bundler,
//! gen-pip, gen-gomod, gen-helm, …) emits this shape; every renderer
//! (gen-nix-incremental, gen-nix-bulk, gen-bazel, …) consumes it;
//! every cache backend (gen-cache-attic, gen-cache-cachix, …) keys
//! on it.
//!
//! See `theory/GEN.md` for the full design — why universal lifting,
//! the 80/20 split, the trait architecture, the milestone plan.
//!
//! ## Shape
//!
//! ```text
//! Manifest (1)
//!   └─ packages: Vec<Package>             (one per crate / gem / wheel / etc.)
//!         ├─ source: PackageSource        (Registry | Git | Path | Local)
//!         ├─ dependencies: Vec<Dependency>
//!         │     ├─ constraint: VersionConstraint
//!         │     ├─ kind: DependencyKind   (Direct | Build | Dev | Optional | Peer)
//!         │     ├─ features_enabled: Vec<String>
//!         │     └─ target_predicate: Option<TargetPredicate>
//!         ├─ features: Vec<Feature>
//!         └─ build_steps: Vec<BuildStep>
//!   └─ lockfile: Option<Lockfile>
//! ```
//!
//! Every adapter is a thin Manifest-producer; every renderer is a
//! thin Manifest-to-Derivation translator. The trait surface stays
//! narrow.

#![allow(clippy::module_name_repetitions)]

pub mod adapter;
pub mod constraint;
pub mod dependency;
pub mod derivation;
pub mod feature;
pub mod lockfile;
pub mod manifest;
pub mod package;
pub mod registry;
pub mod source;
pub mod target;
pub mod version;
pub mod workspace;

pub use adapter::{
    Adapter, AdapterCtx, AdapterError, AdapterQuirkEntry, AdapterResult,
    BuildSpec as AdapterBuildSpec, ConfirmReport, DepChange, DepEdge, DependencyBump, DiffRef,
    DiffReport, InvariantBreak, LockOutcome, Plan, PlanIntent, PlanWarning, PlanWarningSeverity,
    Sbom, SbomFormat,
};
pub use constraint::{Combinator, CompoundConstraint, ConstraintSpec, VersionConstraint};
pub use dependency::{Dependency, DependencyKind};
pub use derivation::{BuildCommand, BuildScript, BuildStep, BuildStepKind, Derivation, DerivationRef};
pub use feature::{Feature, FeatureRef};
pub use lockfile::{ContentHash, Lockfile, ResolvedPackage};
pub use manifest::Manifest;
pub use package::{Package, PackageId};
pub use registry::Registry;
pub use source::PackageSource;
pub use target::{CompoundTargetPredicate, PredicateCombinator, Target, TargetPredicate};
pub use version::Version;
pub use workspace::Workspace;
