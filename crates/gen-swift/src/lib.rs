//! `gen-swift` — swift adapter for the gen ecosystem.
//!
//! Parses `Package.swift` into a typed `BuildSpec` and emits it as
//! Cargo-equivalent typed JSON.

pub mod adapter;
pub mod build_spec;
pub mod error;
pub mod invariants;
pub mod quirks;

pub use adapter::SwiftAdapter;
pub use error::{Result, SwiftError};
