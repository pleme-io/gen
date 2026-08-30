//! `gen-ansible` — ansible adapter for the gen ecosystem.
//!
//! Parses `galaxy.yml` into a typed `BuildSpec` and emits it as
//! Cargo-equivalent typed JSON. See
//! `theory/ECOSYSTEM-INTAKE.md` for the seven-artifact contract.

pub mod adapter;
pub mod build_spec;
pub mod error;
pub mod invariants;
pub mod quirks;

pub use adapter::AnsibleAdapter;
pub use error::{AnsibleError, Result};
