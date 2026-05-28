//! Typed merge strategies for folding per-variant overrides.

/// How a `Dispatcher<V, C, O>` combines successive overrides as
/// it folds over a list of variants.
///
/// The default `OverrideLast` mirrors Nix attrset `//` (later
/// wins). `AccumulateList` mirrors `++` (concatenate). Custom
/// strategies are handled by the consumer's `O` type implementing
/// `Default + std::ops::Add` (or whatever shape fits).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MergeStrategy {
    /// Last variant's override replaces overlapping keys. Default.
    #[default]
    OverrideLast,
    /// Accumulate overrides additively — typically used when `O` is
    /// a `Vec<T>` and each helper returns one element to append.
    Accumulate,
    /// Caller-supplied custom merge — the consumer's `O` type is
    /// responsible for implementing the semantics (e.g. via a
    /// `Monoid` impl).
    Custom,
}
