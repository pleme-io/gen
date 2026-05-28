//! `TypedDispatcher` — introspection trait every typed-variant
//! enum implements via `#[derive(TypedDispatcher)]` in gen-macros.
//!
//! Surface the variant universe (kebab-case serde tags + per-variant
//! field names) so the substrate can mechanically:
//!
//! - emit the matching Nix `helpers = { ... }` table skeleton
//!   (one entry per variant);
//! - render a Lisp catalog entry naming the dispatcher;
//! - generate a coverage test that fails when a variant has no
//!   consumer arm.
//!
//! Trait-only — no runtime dispatch. Rust enums already dispatch
//! natively via `match`; this trait is the *catalog reflection*
//! that exposes the same enum to other runtimes (Nix, Lisp) so they
//! can build dispatch tables against it.
//!
//! ## Naming
//!
//! "Dispatcher" emphasises the catamorphism the trait participates
//! in. See `theory/QUIRK-APPLIER.md` §IV-bis for the full leverage
//! scope — every typed variant universe at a language boundary is a
//! candidate `TypedDispatcher`.

/// Reflection over a typed variant universe — typically a Rust enum
/// declaring `#[serde(tag = "kind", rename_all = "kebab-case")]`.
pub trait TypedDispatcher {
    /// Kebab-case serde tags of every variant in declaration order.
    /// Used by substrate emitters (Nix helpers skeleton, Lisp catalog,
    /// coverage tests) to enumerate the variant universe.
    fn variant_kinds() -> Vec<&'static str>;

    /// Field names per variant, paired with the variant's kebab-case
    /// tag. Used by substrate emitters to produce the matching
    /// `inherit (variant) <fields>` Nix patterns.
    fn variant_fields() -> Vec<(&'static str, Vec<&'static str>)>;

    /// Total variant count. Convenience for coverage assertions.
    #[must_use]
    fn variant_count() -> usize {
        Self::variant_kinds().len()
    }
}
