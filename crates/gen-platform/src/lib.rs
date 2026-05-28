//! `gen-platform` — Rust-first canonical handle for the typed-
//! dispatcher substrate primitive (typed-tagged-union catamorphism
//! at a language boundary). Re-exports `TypedDispatcher` +
//! `#[derive(TypedDispatcher)]`, ships a runtime `Dispatcher<T>`
//! applicator for Rust consumers (controllers, daemons, reactive
//! routers, migration runners, RBAC engines, render passes), and
//! surfaces typed catalog primitives so any pleme-io crate can
//! stand up a typed dispatch surface against any closed variant
//! universe with one cargo add.
//!
//! ## Why a dedicated crate
//!
//! The `mk-typed-dispatcher.nix` primitive proves the value at the
//! Nix dispatch layer. The same shape recurs across pleme-io
//! wherever a closed variant set drives an action (see
//! [`theory/QUIRK-APPLIER.md`](https://github.com/pleme-io/theory/blob/main/QUIRK-APPLIER.md)
//! §IV-bis.1 for the catalog). `gen-platform` is the canonical
//! Rust-side handle so consumers in pangea-operator, engenho
//! controllers, magma plan walkers, anomalia remediations, shinka
//! migrations etc. reach for the same primitive instead of hand-
//! rolling another `match` ladder.
//!
//! ## Surface
//!
//! ```ignore
//! use gen_platform::{Dispatcher, TypedDispatcher};
//!
//! #[derive(serde::Serialize, serde::Deserialize, gen_platform::TypedDispatcher)]
//! #[serde(tag = "kind", rename_all = "kebab-case")]
//! enum Migration {
//!     AddColumn { table: String, column: String },
//!     DropIndex { name: String },
//! }
//!
//! let dispatcher = Dispatcher::<Migration, MyCtx, MyOverride>::new()
//!     .helper("add-column", |variant, ctx| { /* ... */ Default::default() })
//!     .helper("drop-index", |variant, ctx| { /* ... */ Default::default() })
//!     .into_sealed()  // enforces variant coverage
//!     .expect("every variant has a helper");
//!
//! let result = dispatcher.apply(&[migration_a, migration_b], &mut ctx);
//! ```
//!
//! ## Re-exports
//!
//! - `gen_types::TypedDispatcher` — the reflection trait.
//! - `gen_types::DispatcherVariant` — variant envelope shape.
//! - `gen_macros::TypedDispatcher` — `#[derive]` macro.
//!
//! ## New primitives (this crate)
//!
//! - `Dispatcher<V, C, O>` — runtime applicator. Holds typed
//!   helpers per variant kind. `into_sealed()` proves coverage at
//!   construction time (every reflected kind has a registered
//!   helper) — silent unknown variants become impossible.
//! - `SealedDispatcher<V, C, O>` — the post-seal handle; `apply`
//!   cannot fail on coverage anymore (proven at seal time).
//! - `DispatcherError` — typed error for build-time issues
//!   (missing coverage, duplicate helpers).
//! - `MergeStrategy` — how to fold per-variant overrides.

#![allow(clippy::module_name_repetitions)]

pub mod catalog;
pub mod composed;
pub mod dispatcher;
pub mod emit;
pub mod merge;

pub use catalog::{by_label as catalog_by_label, registered as catalog_registered, DispatcherEntry};
pub use composed::{ComposedDispatcher, ComposedSource};
pub use dispatcher::{Dispatcher, DispatcherError, SealedDispatcher};
pub use emit::to_helpers_skeleton;
pub use merge::MergeStrategy;

// Re-exports — single canonical Rust handle.
pub use gen_macros::{BackendError, Discriminant, FromStrKind, IsVariant, TypedDispatcher};
pub use gen_types::{DispatcherVariant, TypedDispatcher as TypedDispatcherTrait};
