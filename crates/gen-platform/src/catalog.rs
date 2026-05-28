//! `DispatcherCatalog` — inventory-style fleet-wide registry of
//! every typed dispatcher in scope. Any pleme-io crate that
//! `#[derive(TypedDispatcher)]`s an enum can register a catalog
//! entry via the [`register_dispatcher!`](crate::register_dispatcher)
//! macro; gen-platform iterates them all at runtime through
//! `DispatcherCatalog::registered()`.
//!
//! This is the catalog reflection layer per `theory/QUIRK-APPLIER.md`
//! §IV-bis.3.e — a single queryable surface for *every place in
//! pleme-io that switches on a typed tag*. Operators query it via
//! `gen dispatchers catalog`; substrate emitters consume it to
//! produce Nix helpers skeletons + Lisp catalog entries
//! mechanically.

/// One entry in the dispatcher catalog. Materialized via
/// [`register_dispatcher!`](crate::register_dispatcher) — the
/// macro emits an `inventory::submit!` block per registered
/// dispatcher.
pub struct DispatcherEntry {
    /// Caller-supplied label (e.g. `"gen.cargo.crate-quirk"`,
    /// `"shinka.migration"`, `"saguao.permission"`). Dot-separated
    /// fleet-wide-unique identifier. Used by operator queries +
    /// substrate emitters.
    pub label: &'static str,
    /// Function returning the variant universe (kebab-case serde
    /// tags). Indirected as a fn pointer so the catalog stays
    /// `'static` while the underlying enum can live anywhere.
    pub variant_kinds: fn() -> Vec<&'static str>,
    /// Function returning per-variant field names. Mirror of
    /// `TypedDispatcher::variant_fields`.
    pub variant_fields: fn() -> Vec<(&'static str, Vec<&'static str>)>,
    /// Function returning the variant count.
    pub variant_count: fn() -> usize,
}

inventory::collect!(DispatcherEntry);

/// Iterate every catalog entry registered fleet-wide. Order is
/// inventory-dependent (not deterministic) — sort by `label` if
/// the consumer needs stable iteration.
#[must_use]
pub fn registered() -> Vec<&'static DispatcherEntry> {
    inventory::iter::<DispatcherEntry>().collect()
}

/// Look up a catalog entry by its dot-separated label.
#[must_use]
pub fn by_label(label: &str) -> Option<&'static DispatcherEntry> {
    inventory::iter::<DispatcherEntry>().find(|e| e.label == label)
}

/// Register a `#[derive(TypedDispatcher)]` enum into the fleet-wide
/// dispatcher catalog. Place at module scope; one call per enum.
///
/// ```ignore
/// use gen_platform::register_dispatcher;
///
/// #[derive(serde::Serialize, gen_platform::TypedDispatcher)]
/// #[serde(tag = "kind", rename_all = "kebab-case")]
/// enum MyMigration { AddColumn { table: String }, DropIndex { name: String } }
///
/// register_dispatcher!("shinka.migration", MyMigration);
/// ```
///
/// After registration, `gen dispatchers catalog` lists the entry
/// + substrate emitters can produce the matching Nix dispatch
/// skeleton without naming `MyMigration` directly.
#[macro_export]
macro_rules! register_dispatcher {
    ($label:expr, $enum_ty:ty) => {
        $crate::catalog::__inventory::submit! {
            $crate::catalog::DispatcherEntry {
                label: $label,
                variant_kinds: || {
                    <$enum_ty as $crate::TypedDispatcherTrait>::variant_kinds()
                },
                variant_fields: || {
                    <$enum_ty as $crate::TypedDispatcherTrait>::variant_fields()
                },
                variant_count: || {
                    <$enum_ty as $crate::TypedDispatcherTrait>::variant_count()
                },
            }
        }
    };
}

/// Public re-export of the inventory crate for the
/// `register_dispatcher!` macro to use. Consumers don't depend on
/// inventory directly — gen-platform owns the dependency edge.
#[doc(hidden)]
pub use ::inventory as __inventory;
