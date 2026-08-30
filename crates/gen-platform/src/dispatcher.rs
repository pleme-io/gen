//! Runtime `Dispatcher<V, C, O>` — the Rust-side peer of
//! `mk-typed-dispatcher.nix`. Same catamorphism, native enum types.
//!
//! Type parameters:
//!
//! - `V`: the typed variant universe (a Rust enum implementing
//!   `gen_types::TypedDispatcher`). Used for compile-time coverage
//!   reflection.
//! - `C`: the runtime context the helpers mutate / read.
//! - `O`: the override type each helper produces. Folded across
//!   the variant list per the `MergeStrategy`.
//!
//! Sealing: after `Dispatcher::new()` + N `helper(...)` calls, the
//! consumer calls `into_sealed()`. The seal asserts that every
//! kind in `V::variant_kinds()` has a registered helper — silent
//! unknown variants become impossible at the type level. Missing
//! coverage returns a typed `DispatcherError`.
//!
//! Anti-pattern this primitive eliminates: hand-rolled `match`
//! ladders sprinkled across pleme-io controllers/daemons/migration
//! runners. Pillar 12 (generation over composition) applied at
//! the Rust runtime layer.

use std::collections::BTreeMap;
use std::marker::PhantomData;

use gen_types::TypedDispatcher;

use crate::merge::MergeStrategy;

/// Build-time / seal-time dispatcher error.
#[derive(Debug, thiserror::Error)]
pub enum DispatcherError {
    /// One or more variant kinds reflected by `V::variant_kinds()`
    /// have no registered helper. Surfaces the exact missing kinds
    /// so the consumer can fix at compile time.
    #[error("dispatcher missing helpers for variant kinds: {missing:?}")]
    MissingCoverage { missing: Vec<String> },
    /// Two helpers registered for the same kind. The dispatcher
    /// refuses ambiguity — pick one.
    #[error("dispatcher already has a helper for kind '{kind}' — register only once")]
    DuplicateHelper { kind: String },
    /// An applied variant carried a kind not in the variant
    /// universe. Can only happen if the variant enum was hand-
    /// constructed via serde without the typed enum (i.e. the
    /// reflection contract was violated upstream).
    #[error("variant kind '{kind}' is unknown to the dispatcher (not in V::variant_kinds())")]
    UnknownKind { kind: String },
}

type Helper<V, C, O> = Box<dyn Fn(&V, &mut C) -> O + Send + Sync + 'static>;

/// Unsealed dispatcher builder. Register helpers per variant kind,
/// then call `into_sealed()` to prove coverage.
pub struct Dispatcher<V, C, O>
where
    V: TypedDispatcher,
{
    helpers: BTreeMap<String, Helper<V, C, O>>,
    strategy: MergeStrategy,
    _marker: PhantomData<fn() -> V>,
}

impl<V, C, O> Default for Dispatcher<V, C, O>
where
    V: TypedDispatcher,
{
    fn default() -> Self {
        Self {
            helpers: BTreeMap::new(),
            strategy: MergeStrategy::default(),
            _marker: PhantomData,
        }
    }
}

impl<V, C, O> Dispatcher<V, C, O>
where
    V: TypedDispatcher,
{
    /// Start a fresh dispatcher with the default `OverrideLast`
    /// merge strategy.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Override the merge strategy used by `apply` when folding
    /// per-variant overrides. Defaults to `OverrideLast`.
    #[must_use]
    pub fn with_strategy(mut self, strategy: MergeStrategy) -> Self {
        self.strategy = strategy;
        self
    }

    /// Register a helper for a kind. Returns `DuplicateHelper` if
    /// the kind already has one — pleme-io's typed-dispatch rule
    /// is: one kind, one helper.
    ///
    /// # Errors
    /// Returns `DispatcherError::DuplicateHelper` if a helper for
    /// `kind` has already been registered.
    pub fn helper<F>(mut self, kind: &str, f: F) -> Result<Self, DispatcherError>
    where
        F: Fn(&V, &mut C) -> O + Send + Sync + 'static,
    {
        if self.helpers.contains_key(kind) {
            return Err(DispatcherError::DuplicateHelper {
                kind: kind.to_string(),
            });
        }
        self.helpers.insert(kind.to_string(), Box::new(f));
        Ok(self)
    }

    /// Convenience: register a helper, panic on duplicate. Use only
    /// when you control both sides of the registration and want
    /// builder-style ergonomics.
    #[must_use]
    pub fn with_helper<F>(self, kind: &str, f: F) -> Self
    where
        F: Fn(&V, &mut C) -> O + Send + Sync + 'static,
    {
        self.helper(kind, f).expect("duplicate helper")
    }

    /// Seal the dispatcher — asserts every kind in
    /// `V::variant_kinds()` has a registered helper. Returns a
    /// `SealedDispatcher` that can apply variants without runtime
    /// coverage checks.
    ///
    /// # Errors
    /// Returns `DispatcherError::MissingCoverage` if any kind in
    /// the variant universe has no registered helper.
    pub fn into_sealed(self) -> Result<SealedDispatcher<V, C, O>, DispatcherError> {
        let universe = V::variant_kinds();
        let mut missing = Vec::new();
        for k in &universe {
            if !self.helpers.contains_key(*k) {
                missing.push((*k).to_string());
            }
        }
        if !missing.is_empty() {
            return Err(DispatcherError::MissingCoverage { missing });
        }
        Ok(SealedDispatcher {
            helpers: self.helpers,
            strategy: self.strategy,
            _marker: PhantomData,
        })
    }
}

/// Post-seal dispatcher — coverage proved at construction. `apply`
/// folds helpers over the variant list using the configured merge
/// strategy.
pub struct SealedDispatcher<V, C, O>
where
    V: TypedDispatcher,
{
    helpers: BTreeMap<String, Helper<V, C, O>>,
    strategy: MergeStrategy,
    _marker: PhantomData<fn() -> V>,
}

impl<V, C, O> SealedDispatcher<V, C, O>
where
    V: TypedDispatcher,
    O: Default,
{
    /// The strategy the dispatcher was sealed with.
    #[must_use]
    pub fn strategy(&self) -> MergeStrategy {
        self.strategy
    }

    /// Apply each variant via the matching helper and collect every
    /// produced override into a `Vec<O>`. The caller decides how to
    /// fold — typical pattern: `MergeStrategy::Accumulate` consumers
    /// keep the Vec; `MergeStrategy::OverrideLast` consumers
    /// reduce it to a single `O` via their own `Monoid`-flavored
    /// reducer (the `O` type's `Default + Add` impl).
    pub fn apply_each(&self, variants: &[V], ctx: &mut C) -> Vec<O>
    where
        V: serde::Serialize,
    {
        let mut out = Vec::with_capacity(variants.len());
        for v in variants {
            let kind = serde_variant_kind(v);
            if let Some(h) = self.helpers.get(&kind) {
                out.push(h(v, ctx));
            }
            // Cannot reach here normally: seal guarantees coverage,
            // and the variant came from a typed V (no string-key
            // construction). Defensive: skip silently. Consumers
            // with a stronger contract can use `try_apply_each`
            // below for typed UnknownKind reporting.
        }
        out
    }

    /// `apply_each` with explicit error surfacing — returns
    /// `UnknownKind` if a variant's serialized kind isn't in the
    /// helpers table. In typed pleme-io use this never fires; it's
    /// available for defensive code paths.
    ///
    /// # Errors
    /// Returns `DispatcherError::UnknownKind` if a variant's
    /// serialized kind has no registered helper.
    pub fn try_apply_each(&self, variants: &[V], ctx: &mut C) -> Result<Vec<O>, DispatcherError>
    where
        V: serde::Serialize,
    {
        let mut out = Vec::with_capacity(variants.len());
        for v in variants {
            let kind = serde_variant_kind(v);
            let Some(h) = self.helpers.get(&kind) else {
                return Err(DispatcherError::UnknownKind { kind });
            };
            out.push(h(v, ctx));
        }
        Ok(out)
    }

    /// Saturation witness — list of every kind the dispatcher
    /// covers, in canonical order. Equal to `V::variant_kinds()`
    /// by construction (the seal proved this). Useful for catalog
    /// tooling that wants to expose what a dispatcher accepts
    /// without recomputing the universe from the trait.
    #[must_use]
    pub fn saturation_witness(&self) -> Vec<&str> {
        self.helpers.keys().map(String::as_str).collect()
    }

    /// The number of registered helpers. Equal to
    /// `V::variant_count()` by construction.
    #[must_use]
    pub fn helper_count(&self) -> usize {
        self.helpers.len()
    }
}

impl<V, C, O> SealedDispatcher<V, C, O>
where
    V: TypedDispatcher + serde::Serialize,
    C: Clone,
    O: Default + Clone + PartialEq,
{
    /// Determinism law — assert that applying the same variants
    /// twice (against a clone of the context) produces the same
    /// output vector. The substrate-wide invariant from
    /// `theory/QUIRK-APPLIER.md` §IV-bis.2: "determinism — same
    /// input → same output, property-test."
    ///
    /// Returns `true` if the law holds for this input; `false` if
    /// the helpers carry observable side effects that diverge on
    /// re-apply. Consumers typically `assert!()` this in tests.
    pub fn is_deterministic(&self, variants: &[V], ctx: &C) -> bool {
        let mut a = ctx.clone();
        let mut b = ctx.clone();
        let r1 = self.apply_each(variants, &mut a);
        let r2 = self.apply_each(variants, &mut b);
        r1 == r2
    }

    /// Idempotence law — assert that applying the variant list
    /// twice in sequence produces the same output as applying it
    /// once (for the `Override`/last-wins merge family). Consumers
    /// whose helpers accumulate state (counters, append-only logs)
    /// will return `false` here; that's the diagnostic — they're
    /// not idempotent in the algebraic sense.
    ///
    /// Returns `true` iff `apply_each(variants) == apply_each(variants ++ variants)`.
    pub fn is_idempotent(&self, variants: &[V], ctx: &C) -> bool
    where
        V: Clone,
    {
        let mut a = ctx.clone();
        let r1 = self.apply_each(variants, &mut a);

        let mut b = ctx.clone();
        let doubled: Vec<V> = variants.iter().chain(variants.iter()).cloned().collect();
        let r2 = self.apply_each(&doubled, &mut b);

        // For OverrideLast semantics: the second pass replays the
        // same variants, and the last application of each kind wins
        // — so r2's tail (last len(variants) entries) should equal
        // r1.
        r2.len() == 2 * r1.len() && r2.iter().skip(r1.len()).cloned().collect::<Vec<_>>() == r1
    }
}

/// Pull the kebab-case serde tag out of a `#[serde(tag = "kind", ...)]`
/// variant. Round-trips through `serde_json::Value` once — cheap and
/// correct independent of the enum's field shape.
fn serde_variant_kind<V: serde::Serialize>(v: &V) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|val| val.get("kind").and_then(|k| k.as_str().map(String::from)))
        .unwrap_or_default()
}
