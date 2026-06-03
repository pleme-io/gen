//! `GenDeltaArtifact` — the cross-ecosystem slim resolver-delta trait.
//!
//! Promoted here (additively) from gen-cargo so EVERY adapter (cargo,
//! gomod, npm, pip, …) shares one trait shape for its `*.gen.lock`
//! artifact. gen-cargo re-exports this trait verbatim under its
//! existing `gen_cargo::gen_delta::GenDeltaArtifact` path, so the cargo
//! consumer + every cargo-path test is byte-identical — nothing in the
//! cargo path changes.
//!
//! A "gen-delta" is the minimal set of resolver facts an ecosystem's
//! own lockfile cannot express, kept in lockstep with that lock via a
//! content hash (the freshness tie). The rust impl was the POC; npm /
//! python / go get the same shape — hence a trait, not a bare fn.

/// A slim, committed resolver-delta: the minimal facts an ecosystem's
/// lockfile cannot express, kept in lockstep with that lock via a
/// content hash.
pub trait GenDeltaArtifact: Sized {
    /// The full in-memory spec this delta is distilled from.
    type FullSpec;
    /// Distillation error type (per-ecosystem).
    type Error: std::error::Error;
    /// Artifact schema version (gates consumer decode).
    const SCHEMA_VERSION: u32;
    /// The committed filename (e.g. `Cargo.gen.lock`, `Go.gen.lock`).
    const FILENAME: &'static str;

    /// Distill the slim delta from the full spec. MUST drop every field
    /// the lockfile already expresses and MUST error rather than emit a
    /// degenerate (empty) delta.
    fn distill(full: &Self::FullSpec) -> Result<Self, Self::Error>;

    /// The freshness tie — equals `builtins.hashFile "sha256"` of the
    /// lock at consume time. Lowercase hex SHA-256.
    fn lock_sha256(&self) -> &str;
}
