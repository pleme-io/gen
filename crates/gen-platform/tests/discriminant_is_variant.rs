//! Exercise `#[derive(Discriminant)]` + `#[derive(IsVariant)]`
//! on a synthetic enum, alongside `#[derive(TypedDispatcher)]`.
//!
//! All three derives target the same closed-variant-universe
//! shape pleme-io uses everywhere — adopting one means the
//! enum gets a typed reflection (variant_kinds + variant_fields)
//! + a stable wire-format discriminant + per-variant predicates,
//! in three lines of derive.
//!
//! See `theory/PATTERN-EXTRACTION.md` Pattern 6 (sibling).

use gen_platform::{Discriminant, IsVariant, TypedDispatcher, TypedDispatcherTrait};
use serde::{Deserialize, Serialize};

/// Synthetic enum mirroring an OTP-style supervisor event.
/// Three derives compose cleanly — no attribute collisions.
#[derive(Clone, Debug, Serialize, Deserialize, Discriminant, IsVariant, TypedDispatcher)]
#[serde(tag = "kind", rename_all = "kebab-case")]
enum SupervisorEvent {
    /// A child started.
    ChildStarted { name: String },
    /// A child died abnormally.
    ChildCrashed { name: String, reason: String },
    /// The supervisor itself is shutting down.
    Shutdown,
}

#[test]
fn discriminant_returns_kebab_case_by_default() {
    let started = SupervisorEvent::ChildStarted { name: "x".into() };
    let crashed = SupervisorEvent::ChildCrashed {
        name: "y".into(),
        reason: "oom".into(),
    };
    let shutdown = SupervisorEvent::Shutdown;

    assert_eq!(started.discriminant(), "child-started");
    assert_eq!(crashed.discriminant(), "child-crashed");
    assert_eq!(shutdown.discriminant(), "shutdown");
}

#[test]
fn is_variant_predicates_per_variant() {
    let started = SupervisorEvent::ChildStarted { name: "x".into() };
    let crashed = SupervisorEvent::ChildCrashed {
        name: "y".into(),
        reason: "oom".into(),
    };
    let shutdown = SupervisorEvent::Shutdown;

    assert!(started.is_child_started());
    assert!(!started.is_child_crashed());
    assert!(!started.is_shutdown());

    assert!(crashed.is_child_crashed());
    assert!(!crashed.is_child_started());
    assert!(!crashed.is_shutdown());

    assert!(shutdown.is_shutdown());
    assert!(!shutdown.is_child_started());
    assert!(!shutdown.is_child_crashed());
}

#[test]
fn typed_dispatcher_reflection_alongside_discriminant() {
    // Both derives produce a coherent variant universe — the kinds
    // from the dispatcher reflection match the discriminants from
    // the same enum.
    let kinds = SupervisorEvent::variant_kinds();
    assert_eq!(kinds, vec!["child-started", "child-crashed", "shutdown"]);
    assert_eq!(SupervisorEvent::variant_count(), 3);

    let samples = [
        SupervisorEvent::ChildStarted { name: "a".into() },
        SupervisorEvent::ChildCrashed {
            name: "b".into(),
            reason: "c".into(),
        },
        SupervisorEvent::Shutdown,
    ];
    for sample in samples {
        assert!(
            kinds.contains(&sample.discriminant()),
            "discriminant {} must appear in variant_kinds",
            sample.discriminant()
        );
    }
}

#[test]
fn is_variant_is_const_fn_usable_in_const_context() {
    // Unit variants can be used in const context (no Drop fields).
    // This proves Discriminant + IsVariant emit real `const fn`.
    const IS_SHUTDOWN: bool = SupervisorEvent::Shutdown.is_shutdown();
    const SHUTDOWN_KIND: &str = SupervisorEvent::Shutdown.discriminant();
    assert!(IS_SHUTDOWN);
    assert_eq!(SHUTDOWN_KIND, "shutdown");
}
