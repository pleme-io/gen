//! `gen-helm` — helm adapter for the gen ecosystem.
//!
//! Parses `Chart.yaml` into a typed `BuildSpec` and emits it as
//! Cargo-equivalent typed JSON.

pub mod adapter;
pub mod build_spec;
pub mod error;
pub mod invariants;
pub mod quirks;

pub use adapter::HelmAdapter;
pub use error::{HelmError, Result};
