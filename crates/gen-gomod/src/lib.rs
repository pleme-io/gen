//! `gen-gomod` — gomod adapter for the gen ecosystem.
//!
//! Parses `go.mod` + a vendored tree (via `go list -deps -json`) into a
//! typed per-package [`build_spec::BuildSpec`] (v2 incremental) and emits
//! it as `Go.build-spec.json`. See `theory/ECOSYSTEM-INTAKE.md` for the
//! seven-artifact contract and the M1 build doc for the per-package
//! (rustc-per-crate-in-Go) delta.
//!
//! The TYPED-SPEC + INTERPRETER TRIPLET:
//! - **Rust border** — [`build_spec`] (the v2 shape + the v1 `coarse` submod).
//! - **Lisp spec** — `specs/go-package-build.lisp` + `specs/adapter.lisp`.
//! - **Interpreter** — [`interp`] (`apply(env, ctx) -> BuildSpec`), with
//!   every side effect behind the [`interp::GoBuildEnv`] trait.

pub mod adapter;
pub mod build_spec;
pub mod emit;
pub mod error;
pub mod gen_delta;
pub mod golist;
pub mod gomod;
pub mod interp;
pub mod invariants;
pub mod quirks;
pub mod testkit;

pub use adapter::GomodAdapter;
pub use build_spec::{BuildSpec, PackageKind, PackageSpec, SCHEMA_VERSION};
pub use emit::{generate_and_write, generate_for_target_and_write, SPEC_FILENAME};
pub use error::{GomodError, Result};
pub use interp::{apply, EncodeCtx, GoBuildEnv, RealGoBuildEnv};
