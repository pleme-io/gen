//! `gen-gomod` — gomod adapter for the gen ecosystem.
//!
//! Parses `go.mod` into a typed `BuildSpec` and emits it as
//! Cargo-equivalent typed JSON. See
//! `theory/ECOSYSTEM-INTAKE.md` for the seven-artifact contract.

pub mod adapter;
pub mod build_spec;
pub mod error;
pub mod gen_delta;
pub mod invariants;
pub mod parse;
pub mod quirks;
pub mod vendor_prefetcher;

pub use adapter::GomodAdapter;
pub use error::{GomodError, Result};
