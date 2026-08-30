//! `ComposedDispatcher` — closure-under-composition for the typed
//! dispatcher catamorphism. Given N sealed dispatchers over
//! disjoint variant universes, produce a single dispatcher that
//! handles any variant from any of them by serde-tag dispatch.
//!
//! Proves the algebraic law from `theory/QUIRK-APPLIER.md`
//! §IV-bis.3.d: "the combinator is closed under composition;
//! polyglot caixa packages (Rust+Python+Helm) compose multiple
//! ecosystem appliers in sequence into a unified override stack."
//!
//! ## Type erasure
//!
//! The composed view operates on `serde_json::Value` variants
//! (each carrying a `"kind"` field). Source dispatchers stay
//! typed (`SealedDispatcher<V, C, O>`); composition walks the
//! per-kind helpers through a thin adapter that round-trips
//! variants through serde_json. This is the same shape the
//! substrate's `mk-typed-dispatcher.nix` consumes (untyped JSON
//! values keyed by `kind`), so composed Rust dispatchers and
//! composed Nix dispatchers share semantics by construction.

use std::collections::BTreeMap;

use crate::dispatcher::{DispatcherError, SealedDispatcher};
use crate::merge::MergeStrategy;
use gen_types::TypedDispatcher;

/// One per-kind helper as installed in the composed dispatcher.
/// Box of Fn over the untyped JSON variant + context.
type ComposedHelper<C, O> = Box<dyn Fn(&serde_json::Value, &mut C) -> O + Send + Sync + 'static>;

/// Composition of multiple `SealedDispatcher`s over disjoint
/// variant universes. Operates on `serde_json::Value` variants
/// keyed by the `"kind"` field. Use `.add(...)` to extend.
pub struct ComposedDispatcher<C, O> {
    helpers: BTreeMap<String, ComposedHelper<C, O>>,
    strategy: MergeStrategy,
    /// Per-source labels mapped to the set of kinds they contribute.
    /// Surfaces the composition shape for diagnostic queries
    /// without requiring callers to track the input dispatchers.
    sources: Vec<ComposedSource>,
}

/// One source dispatcher's contribution to the composition.
#[derive(Debug, Clone)]
pub struct ComposedSource {
    /// Caller-supplied label (e.g. `"gen.cargo"`, `"gen.helm"`).
    pub label: String,
    /// Kinds the source contributed.
    pub kinds: Vec<String>,
}

impl<C, O> Default for ComposedDispatcher<C, O> {
    fn default() -> Self {
        Self {
            helpers: BTreeMap::new(),
            strategy: MergeStrategy::default(),
            sources: Vec::new(),
        }
    }
}

impl<C, O> ComposedDispatcher<C, O>
where
    O: Default,
{
    /// Start a fresh composition with the default strategy.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Override the merge strategy.
    #[must_use]
    pub fn with_strategy(mut self, strategy: MergeStrategy) -> Self {
        self.strategy = strategy;
        self
    }

    /// The strategy this composition uses.
    #[must_use]
    pub fn strategy(&self) -> MergeStrategy {
        self.strategy
    }

    /// Extend with another sealed dispatcher. Kinds must be
    /// disjoint from any already-added source — overlap raises
    /// `DuplicateHelper` (which here means "the composition's
    /// kind universes overlap; pick one source for each kind").
    ///
    /// `label` is a fleet-unique tag for the source (typically
    /// matches the `register_dispatcher!` label).
    ///
    /// # Errors
    /// Returns `DispatcherError::DuplicateHelper` if any of the
    /// added dispatcher's kinds is already covered.
    pub fn add<V>(
        mut self,
        label: &str,
        source: SealedDispatcher<V, C, O>,
    ) -> Result<Self, DispatcherError>
    where
        V: TypedDispatcher + serde::Serialize + serde::de::DeserializeOwned + 'static,
        C: 'static,
        O: 'static,
    {
        let kinds = source
            .saturation_witness()
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>();
        for k in &kinds {
            if self.helpers.contains_key(k) {
                return Err(DispatcherError::DuplicateHelper { kind: k.clone() });
            }
        }
        // Convert the source's typed helpers into untyped JSON
        // helpers. One closure per kind that round-trips
        // serde_json::Value into V and dispatches through the
        // typed dispatcher.
        let arc = std::sync::Arc::new(source);
        for k in &kinds {
            let arc2 = arc.clone();
            let kind_owned = k.clone();
            self.helpers.insert(
                kind_owned,
                Box::new(move |value: &serde_json::Value, ctx: &mut C| {
                    let typed: V = match serde_json::from_value(value.clone()) {
                        Ok(t) => t,
                        Err(_) => return O::default(),
                    };
                    let r = arc2.apply_each(&[typed], ctx);
                    r.into_iter().next().unwrap_or_default()
                }),
            );
        }
        self.sources.push(ComposedSource {
            label: label.to_string(),
            kinds,
        });
        Ok(self)
    }

    /// Apply each `serde_json::Value` variant against the
    /// composed helpers. Unknown kinds skip silently — use
    /// `try_apply_each` for typed `UnknownKind` reporting.
    pub fn apply_each(&self, variants: &[serde_json::Value], ctx: &mut C) -> Vec<O>
    where
        O: Default,
    {
        let mut out = Vec::with_capacity(variants.len());
        for v in variants {
            let Some(kind) = v.get("kind").and_then(|k| k.as_str()) else {
                continue;
            };
            if let Some(h) = self.helpers.get(kind) {
                out.push(h(v, ctx));
            }
        }
        out
    }

    /// `apply_each` with explicit error surfacing.
    ///
    /// # Errors
    /// Returns `DispatcherError::UnknownKind` if a variant's kind
    /// has no registered helper across the composition.
    pub fn try_apply_each(
        &self,
        variants: &[serde_json::Value],
        ctx: &mut C,
    ) -> Result<Vec<O>, DispatcherError>
    where
        O: Default,
    {
        let mut out = Vec::with_capacity(variants.len());
        for v in variants {
            let kind = v.get("kind").and_then(|k| k.as_str()).ok_or_else(|| {
                DispatcherError::UnknownKind {
                    kind: "<no kind field>".to_string(),
                }
            })?;
            let Some(h) = self.helpers.get(kind) else {
                return Err(DispatcherError::UnknownKind {
                    kind: kind.to_string(),
                });
            };
            out.push(h(v, ctx));
        }
        Ok(out)
    }

    /// Number of helpers in the composition (sum of source
    /// helpers; equal to the disjoint union of source variant
    /// universes).
    #[must_use]
    pub fn helper_count(&self) -> usize {
        self.helpers.len()
    }

    /// Read-only view of every kind covered by the composition,
    /// in canonical sorted order.
    #[must_use]
    pub fn covered_kinds(&self) -> Vec<&str> {
        self.helpers.keys().map(String::as_str).collect()
    }

    /// Read-only view of the sources contributing to the
    /// composition (label + kinds each contributed).
    #[must_use]
    pub fn sources(&self) -> &[ComposedSource] {
        &self.sources
    }
}
