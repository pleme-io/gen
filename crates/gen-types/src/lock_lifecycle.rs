//! Substrate-wide `LockLifecyclePrimitive` trait.
//!
//! The deterministic-lock pattern (transient build artifacts +
//! explicit operator snapshots + typed diffs) generalizes across
//! every package-manager adapter. cargo today, npm/bundler/poetry/
//! gomod/helm tomorrow. This trait is the typed contract: implement
//! these methods, get the `gen lock` CLI surface + substrate
//! refusal-on-drift gating for free.
//!
//! ## Required substrate-side guarantees
//!
//! An implementor promises:
//!
//! 1. **`State` is a closed typed enum** with at least the canonical
//!    four states — `Unlocked` / `Locked` / `Drifted` / `MissingLock`.
//!    Concrete adapters may add more (e.g. `gen-npm` might add
//!    `WorkspaceProtocolMismatch`). The state IS observable from
//!    filesystem alone (no daemons, no network, no cached state).
//!
//! 2. **`LockDiff` is a structured value type** (not text). Two
//!    consecutive `update` calls with no Cargo.lock/package-lock.json
//!    movement between produce byte-equal diffs.
//!
//! 3. **`current_state` is pure** — same filesystem state → same
//!    output. Idempotent and deterministic.
//!
//! 4. **`snapshot` / `update` / `reset` are explicit operator verbs**
//!    — they NEVER fire as a side-effect of `current_state` or any
//!    other read.
//!
//! ## Trait-boundary testing
//!
//! Each implementation provides a parallel mock impl (e.g.
//! `MockCargoLifecycle` for tests) that lets gen-cli's dispatcher
//! exercise every state transition hermetically — same shape as
//! `PathDepResolver` + `MetadataSource` already use.

use std::path::Path;

use serde::{Serialize, de::DeserializeOwned};

/// Typed lock-lifecycle primitive — one impl per adapter.
///
/// Implementors expose the deterministic-lock contract to
/// substrate's lockfile-builder + gen-cli's `lock` subcommand.
/// Concrete states are adapter-specific; the trait constrains
/// only what substrate needs to dispatch on.
pub trait LockLifecyclePrimitive: Send + Sync {
    /// Adapter-specific state enum. Must include the canonical
    /// four substrates (Unlocked / Locked / Drifted / MissingLock)
    /// reachable via the `is_*` methods below. Each adapter is free
    /// to add more states (workspace-protocol drift, registry-pin
    /// staleness, etc.) — substrate dispatches via the canonical
    /// flags so extra states don't break the gate.
    type State: Clone + Serialize + DeserializeOwned + Send + Sync + 'static;

    /// Adapter-specific structural diff. Substrate emitters
    /// (release-notes builders, PR-body renderers) consume this
    /// type directly.
    type Diff: Clone + Serialize + DeserializeOwned + Send + Sync + 'static;

    /// Short ecosystem identifier (e.g. `"cargo"`, `"npm"`). Used
    /// for routing by gen-cli + substrate emitters.
    fn ecosystem(&self) -> &'static str;

    /// Read the current state from the filesystem. Pure — same
    /// inputs always yield the same output. No subprocess calls
    /// other than hashing the lockfile.
    fn current_state(&self, root: &Path) -> Self::State;

    /// Adapter-canonical projection: does this state require explicit
    /// operator action before substrate can build?
    fn requires_operator_action(&self, state: &Self::State) -> bool;

    /// Adapter-canonical projection: is this state the byte-equal
    /// "committed snapshot matches current source" condition?
    fn is_locked(&self, state: &Self::State) -> bool;

    /// Adapter-canonical projection: is the source-side lockfile
    /// (Cargo.lock / package-lock.json / etc.) missing?
    fn is_missing_lock(&self, state: &Self::State) -> bool;

    /// Explicit operator snapshot — write the committed lock
    /// artifact (Cargo.build-spec.json / equivalent). Errors when
    /// the state is `MissingLock` or when the snapshot can't be
    /// written.
    fn snapshot(&self, root: &Path) -> Result<(), LockError>;

    /// Explicit operator update — regenerate the committed lock +
    /// compute the typed diff against the previous snapshot.
    /// Returns the typed diff so callers can render it.
    fn update(&self, root: &Path) -> Result<Self::Diff, LockError>;

    /// Explicit operator reset — delete the committed lock artifact
    /// (return to the `Unlocked` state). Errors only on filesystem
    /// failure (a missing artifact is treated as success — idempotent).
    fn reset(&self, root: &Path) -> Result<(), LockError>;
}

/// Typed errors emitted by any `LockLifecyclePrimitive`. Adapters
/// that need richer context wrap their domain errors in `Source`.
#[derive(Debug, thiserror::Error)]
pub enum LockError {
    /// Workspace lockfile (Cargo.lock / package-lock.json / etc.)
    /// is missing — operator must run the ecosystem's bootstrap
    /// before any lock action.
    #[error("workspace lockfile missing at {}; run the ecosystem's bootstrap (`cargo generate-lockfile`, `npm install`, etc.) first", path.display())]
    MissingLockfile { path: std::path::PathBuf },
    /// Spec generation failed — wrap the adapter's domain error.
    #[error("lock action `{action}` failed: {source}")]
    SpecGeneration {
        action: &'static str,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// Filesystem error (read/write/delete).
    #[error("filesystem error during lock action `{action}` at {}: {source}", path.display())]
    Io {
        action: &'static str,
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    /// Hermetic mock implementor — pure lock-free state via
    /// `AtomicU8`. No Mutex, no poison, no panic path. Production
    /// adapter integration tests follow the same shape: no shared
    /// mutable state that can fail to acquire.
    #[derive(Default)]
    struct MockLifecycle {
        state: std::sync::atomic::AtomicU8,
    }

    // State encoding — small, totally typed, no failure mode for
    // load/store.
    const S_UNLOCKED: u8 = 0;
    const S_LOCKED: u8 = 1;
    const S_DRIFTED: u8 = 2;
    const S_MISSING_LOCK: u8 = 3;

    #[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(tag = "kind", rename_all = "kebab-case")]
    enum MockState {
        #[default]
        Unlocked,
        Locked,
        Drifted,
        MissingLock,
    }

    impl MockState {
        const fn encode(&self) -> u8 {
            match self {
                MockState::Unlocked => S_UNLOCKED,
                MockState::Locked => S_LOCKED,
                MockState::Drifted => S_DRIFTED,
                MockState::MissingLock => S_MISSING_LOCK,
            }
        }
        const fn decode(byte: u8) -> Self {
            match byte {
                S_LOCKED => MockState::Locked,
                S_DRIFTED => MockState::Drifted,
                S_MISSING_LOCK => MockState::MissingLock,
                _ => MockState::Unlocked,
            }
        }
    }

    impl MockLifecycle {
        fn with_state(s: MockState) -> Self {
            Self {
                state: std::sync::atomic::AtomicU8::new(s.encode()),
            }
        }
        fn store(&self, s: MockState) {
            self.state
                .store(s.encode(), std::sync::atomic::Ordering::SeqCst);
        }
        fn load(&self) -> MockState {
            MockState::decode(self.state.load(std::sync::atomic::Ordering::SeqCst))
        }
    }

    #[derive(Clone, Default, Serialize, Deserialize)]
    struct MockDiff(usize);

    impl LockLifecyclePrimitive for MockLifecycle {
        type State = MockState;
        type Diff = MockDiff;
        fn ecosystem(&self) -> &'static str {
            "mock"
        }
        fn current_state(&self, _: &Path) -> Self::State {
            self.load()
        }
        fn requires_operator_action(&self, s: &Self::State) -> bool {
            matches!(s, MockState::Drifted | MockState::MissingLock)
        }
        fn is_locked(&self, s: &Self::State) -> bool {
            matches!(s, MockState::Locked)
        }
        fn is_missing_lock(&self, s: &Self::State) -> bool {
            matches!(s, MockState::MissingLock)
        }
        fn snapshot(&self, _: &Path) -> Result<(), LockError> {
            self.store(MockState::Locked);
            Ok(())
        }
        fn update(&self, _: &Path) -> Result<Self::Diff, LockError> {
            self.store(MockState::Locked);
            Ok(MockDiff(0))
        }
        fn reset(&self, _: &Path) -> Result<(), LockError> {
            self.store(MockState::Unlocked);
            Ok(())
        }
    }

    #[test]
    fn mock_round_trips_state_transitions() {
        let m = MockLifecycle::default();
        let path = std::path::Path::new("/tmp/anything");
        assert_eq!(m.current_state(path), MockState::Unlocked);
        m.snapshot(path).unwrap();
        assert_eq!(m.current_state(path), MockState::Locked);
        assert!(m.is_locked(&MockState::Locked));
        m.reset(path).unwrap();
        assert_eq!(m.current_state(path), MockState::Unlocked);
    }

    #[test]
    fn canonical_projections_classify_states_correctly() {
        let m = MockLifecycle::default();
        assert!(!m.requires_operator_action(&MockState::Unlocked));
        assert!(!m.requires_operator_action(&MockState::Locked));
        assert!(m.requires_operator_action(&MockState::Drifted));
        assert!(m.requires_operator_action(&MockState::MissingLock));
        assert!(m.is_missing_lock(&MockState::MissingLock));
        assert!(!m.is_missing_lock(&MockState::Unlocked));
    }

    #[test]
    fn ecosystem_identifier_is_static() {
        let m = MockLifecycle::default();
        assert_eq!(m.ecosystem(), "mock");
    }
}
