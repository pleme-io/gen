//! Typed lock-lifecycle primitive for transient cargo locks.
//!
//! **The deterministic-lock pattern.** The repository never carries
//! a committed `Cargo.build-spec.json` by default — it's a build-
//! time artifact, not source. Operators explicitly snapshot the
//! lock via `gen lock` when they want a stable, reviewable
//! point-in-time pin (e.g. for a release). Between snapshots,
//! substrate's IFD regenerates the spec from the current
//! `Cargo.lock` via the sha256 freshness gate.
//!
//! Every change to the lock is therefore:
//!   - explicit (operator-driven),
//!   - typed (the diff is a structured value, not a git noise diff),
//!   - reviewable (the diff lists `<pkg>@<from> → <to>` for every
//!     moved dep + every added/removed source).
//!
//! ## State machine
//!
//! ```text
//!                                                     gen lock
//!     ┌───────────────┐   gen lock --reset   ┌───────────────┐
//!     │   Unlocked    │ ◄─────────────────── │    Locked     │
//!     │ (no spec on   │                       │ (spec on disk │
//!     │  disk; IFD    │ ───────────────────► │  + matches    │
//!     │  regens)      │      gen lock         │  Cargo.lock)  │
//!     └───────┬───────┘                       └───────┬───────┘
//!             │                                       │ Cargo.lock
//!             │ Cargo.lock                            │ moves
//!             │ missing                               ▼
//!             ▼                              ┌───────────────┐
//!     ┌───────────────┐                      │   Drifted     │
//!     │ MissingLock   │                      │ (committed    │
//!     │ (no Cargo.lock│                      │  ≠ current)   │
//!     │  buildable)   │                      └───────┬───────┘
//!     └───────────────┘                              │
//!                                                    │ gen lock --update
//!                                                    │ (emits typed LockDiff)
//!                                                    ▼
//!                                            ┌───────────────┐
//!                                            │    Locked     │
//!                                            │ (refreshed)   │
//!                                            └───────────────┘
//! ```
//!
//! ## Cross-adapter design
//!
//! The state enum (`LockLifecycleState`) is cargo-specific today
//! but the SHAPE — Unlocked / Locked / Drifted / MissingLock — is
//! the universal lock lifecycle. When gen-npm / gen-bundler /
//! gen-poetry land their `build` impls, each emits its own
//! `<eco>LockLifecycleState` with the same four states. Substrate
//! emitters then dispatch uniformly across adapters via the typed
//! discriminant.
//!
//! ## Determinism guarantee
//!
//! - Same `Cargo.lock` content + same gen-cargo version → byte-equal
//!   `Cargo.build-spec.json`. Enforced by the existing freshness gate
//!   (SHA-256 over Cargo.lock).
//! - `LockDiff` is computed by structural comparison of two
//!   `BuildSpec`s. Same inputs → same diff. No textual heuristics.
//! - The FSM has no hidden state — every transition is observable
//!   from filesystem state alone.

use crate::build_spec::{BuildSpec, CrateSource, SCHEMA_VERSION};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Typed lifecycle state of a cargo transient lock. Closed enum —
/// every variant is observable from the filesystem.
///
/// Substrate's IFD path consults this to decide regen vs reuse:
/// `Unlocked` → IFD regen; `Locked` → reuse committed spec;
/// `Drifted` → REFUSE the build with a typed error pointing the
/// operator at `gen lock --update`; `MissingLock` → can't build at
/// all.
#[gen_macros::fsm(label = "gen.cargo.lock-lifecycle-state")]
pub enum LockLifecycleState {
    /// No `Cargo.build-spec.json` on disk. The repository is in
    /// transient-lock mode; substrate IFD regenerates the spec at
    /// build time from the current `Cargo.lock`. Default state for
    /// repos that never commit their spec (the new fleet shape).
    Unlocked {
        /// SHA-256 of the workspace's current `Cargo.lock`. Acts as
        /// the deterministic identity of the "current" build state.
        current_lock_hash: String,
    },
    /// `Cargo.build-spec.json` exists AND its embedded
    /// `cargo_lock_sha256` matches the current `Cargo.lock`'s
    /// digest. The committed spec is byte-equal to what a regen
    /// would produce; substrate IFD bypasses regen entirely.
    Locked {
        spec_hash: String,
        lock_hash: String,
    },
    /// `Cargo.build-spec.json` exists but its `cargo_lock_sha256`
    /// disagrees with the current `Cargo.lock`. The committed lock
    /// is STALE — building with it would produce a different
    /// resolve than `cargo build` locally would. Operator must run
    /// `gen lock --update` to refresh (and review the
    /// resulting `LockDiff`) or `gen lock --reset` to drop the
    /// committed lock entirely.
    Drifted {
        committed_lock_hash: String,
        current_lock_hash: String,
    },
    /// `Cargo.lock` is missing — the workspace isn't in a buildable
    /// state. Operator runs `cargo generate-lockfile` to recover.
    MissingLock,
}

impl LockLifecycleState {
    /// Operator-facing one-line summary.
    #[must_use]
    pub fn summary(&self) -> &'static str {
        match self {
            Self::Unlocked { .. } => "unlocked",
            Self::Locked { .. } => "locked",
            Self::Drifted { .. } => "drifted",
            Self::MissingLock => "missing-lock",
        }
    }

    /// True iff substrate's nix-side build can proceed without
    /// IFD-regen.
    #[must_use]
    pub fn can_build_with_committed_spec(&self) -> bool {
        matches!(self, Self::Locked { .. })
    }

    /// True iff an operator action is required before a deterministic
    /// build is possible.
    #[must_use]
    pub fn requires_operator_action(&self) -> bool {
        matches!(self, Self::Drifted { .. } | Self::MissingLock)
    }
}

/// Compute the lifecycle state from filesystem alone — pure read,
/// two SHA-256 hashes worst case. Same input → same output.
#[must_use]
pub fn current_state(root: &Path) -> LockLifecycleState {
    use crate::build_spec::{check_freshness, Freshness};
    match check_freshness(root) {
        Freshness::MissingLock => LockLifecycleState::MissingLock,
        Freshness::MissingSpec { lock_hash } | Freshness::UnhashedSpec { lock_hash } => {
            LockLifecycleState::Unlocked {
                current_lock_hash: lock_hash,
            }
        }
        Freshness::Fresh { spec_hash, lock_hash } => LockLifecycleState::Locked {
            spec_hash,
            lock_hash,
        },
        Freshness::Drifted { spec_hash, lock_hash } => LockLifecycleState::Drifted {
            committed_lock_hash: spec_hash,
            current_lock_hash: lock_hash,
        },
    }
}

// ── LockDiff: typed structural diff between two BuildSpecs ──────────

/// Typed structural diff. Every field is a closed set of typed
/// records — substrate emitters consume the diff directly to render
/// release notes, audit logs, or PR-body summaries.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockDiff {
    /// Added crates — present in `new` but not in `old`.
    pub added: Vec<LockEntry>,
    /// Removed crates — present in `old` but not in `new`.
    pub removed: Vec<LockEntry>,
    /// Crates whose version moved.
    pub upgraded: Vec<LockUpgrade>,
    /// Crates whose source kind or URL changed (e.g. registry → git,
    /// or git rev moved without a version bump).
    pub source_changed: Vec<LockSourceChange>,
    /// Schema-version delta (rare; load-bearing for migration audits).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<SchemaVersionDelta>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockEntry {
    pub name: String,
    pub version: String,
    pub source_kind: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockUpgrade {
    pub name: String,
    pub from_version: String,
    pub to_version: String,
    pub source_kind: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockSourceChange {
    pub name: String,
    pub version: String,
    pub from_kind: String,
    pub to_kind: String,
    pub from_locator: String,
    pub to_locator: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemaVersionDelta {
    pub from: u32,
    pub to: u32,
}

impl LockDiff {
    /// True iff no entries changed at all. Same inputs always yield
    /// the same answer.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.added.is_empty()
            && self.removed.is_empty()
            && self.upgraded.is_empty()
            && self.source_changed.is_empty()
            && self.schema_version.is_none()
    }

    /// Total count of moved entries — for at-a-glance reporting.
    #[must_use]
    pub fn total_moves(&self) -> usize {
        self.added.len() + self.removed.len() + self.upgraded.len() + self.source_changed.len()
    }
}

/// Pure-function structural diff. Deterministic — same inputs yield
/// byte-equal output. Sorts every output list by crate name so the
/// emitted JSON is reviewable like a PR diff.
#[must_use]
pub fn diff_specs(old: &BuildSpec, new: &BuildSpec) -> LockDiff {
    use std::collections::BTreeMap;

    // Index by crate name. Multiple versions of the same crate are
    // possible (cargo resolver allows side-by-side); we key by
    // (name, version) for the value lookup but group changes by
    // crate name for the operator-facing diff.
    let mut old_by_id: BTreeMap<(String, String), &crate::build_spec::CrateSpec> =
        BTreeMap::new();
    for c in old.crates.values() {
        old_by_id.insert((c.name.clone(), c.version.clone()), c);
    }
    let mut new_by_id: BTreeMap<(String, String), &crate::build_spec::CrateSpec> =
        BTreeMap::new();
    for c in new.crates.values() {
        new_by_id.insert((c.name.clone(), c.version.clone()), c);
    }

    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut upgraded: Vec<LockUpgrade> = Vec::new();
    let mut source_changed = Vec::new();
    let mut pending_unmatched_new: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();

    // 1. Same (name, version) in both → check for source change.
    // 2. Same name, different version → potential upgrade (handled
    //    after the first pass).
    // 3. Only in new → added.
    // 4. Only in old → removed.

    for ((name, version), new_crate) in &new_by_id {
        if let Some(old_crate) = old_by_id.get(&(name.clone(), version.clone())) {
            // Same name+version. Did the source change?
            let (old_kind, old_loc) = source_kind_and_locator(&old_crate.source);
            let (new_kind, new_loc) = source_kind_and_locator(&new_crate.source);
            if old_kind != new_kind || old_loc != new_loc {
                source_changed.push(LockSourceChange {
                    name: name.clone(),
                    version: version.clone(),
                    from_kind: old_kind,
                    to_kind: new_kind,
                    from_locator: old_loc,
                    to_locator: new_loc,
                });
            }
        } else {
            pending_unmatched_new
                .entry(name.clone())
                .or_default()
                .push((version.clone(), source_kind(&new_crate.source)));
        }
    }

    // For each name with unmatched new versions, check if old also
    // has that name — if so it's an upgrade; otherwise an add.
    for (name, new_versions) in pending_unmatched_new {
        let old_versions: Vec<String> = old_by_id
            .iter()
            .filter(|((n, _), _)| n == &name)
            .map(|((_, v), _)| v.clone())
            .collect();
        if old_versions.is_empty() {
            for (v, kind) in new_versions {
                added.push(LockEntry {
                    name: name.clone(),
                    version: v,
                    source_kind: kind,
                });
            }
        } else {
            for (new_v, kind) in &new_versions {
                for old_v in &old_versions {
                    upgraded.push(LockUpgrade {
                        name: name.clone(),
                        from_version: old_v.clone(),
                        to_version: new_v.clone(),
                        source_kind: kind.clone(),
                    });
                }
            }
        }
    }

    // Removed: names+versions in old not in new (AND no other
    // version of that name in new — those are upgrades, not
    // removals).
    for ((name, version), c) in &old_by_id {
        if !new_by_id.contains_key(&(name.clone(), version.clone())) {
            let same_name_in_new = new_by_id.iter().any(|((n, _), _)| n == name);
            if !same_name_in_new {
                removed.push(LockEntry {
                    name: name.clone(),
                    version: version.clone(),
                    source_kind: source_kind(&c.source),
                });
            }
        }
    }

    let mut diff = LockDiff {
        added,
        removed,
        upgraded,
        source_changed,
        schema_version: if old.version != new.version {
            Some(SchemaVersionDelta {
                from: old.version,
                to: new.version,
            })
        } else {
            None
        },
    };

    diff.added.sort_by(|a, b| (a.name.clone(), a.version.clone()).cmp(&(b.name.clone(), b.version.clone())));
    diff.removed.sort_by(|a, b| (a.name.clone(), a.version.clone()).cmp(&(b.name.clone(), b.version.clone())));
    diff.upgraded.sort_by(|a, b| a.name.cmp(&b.name));
    diff.source_changed.sort_by(|a, b| a.name.cmp(&b.name));
    diff
}

fn source_kind(s: &CrateSource) -> String {
    match s {
        CrateSource::Registry { .. } => "registry".to_string(),
        CrateSource::Git { .. } => "git".to_string(),
        CrateSource::Path { .. } => "path".to_string(),
    }
}

fn source_kind_and_locator(s: &CrateSource) -> (String, String) {
    match s {
        CrateSource::Registry { url, .. } => ("registry".to_string(), url.clone()),
        CrateSource::Git { url, rev, .. } => ("git".to_string(), format!("{url}#{rev}")),
        CrateSource::Path { relative_path } => ("path".to_string(), relative_path.clone()),
    }
}

#[allow(dead_code)]
const _: u32 = SCHEMA_VERSION;

// Note: typed-dispatcher catalog registration is emitted by the
// `#[gen_macros::fsm(label = "...")]` attribute on the enum above.
// No standalone `gen_platform::register_dispatcher!` call needed.

// ── LockLifecyclePrimitive trait impl ───────────────────────────────
//
// Implements gen_types::LockLifecyclePrimitive so gen-cli's `gen lock`
// + substrate's strictTransientLock gate dispatch uniformly across
// every adapter that ships the same trait impl. Future adapters
// (gen-npm, gen-bundler, gen-poetry) follow this exact pattern: one
// `impl` block, all four operator verbs + substrate refusal-on-drift
// wired for free.

/// Cargo adapter for the `LockLifecyclePrimitive` trait. Zero-sized
/// struct — all state is filesystem-derived, no instance state to
/// carry. Construct via `CargoLifecycle::new()`.
#[derive(Debug, Default, Clone, Copy)]
pub struct CargoLifecycle;

impl CargoLifecycle {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl gen_types::LockLifecyclePrimitive for CargoLifecycle {
    type State = LockLifecycleState;
    type Diff = LockDiff;

    fn ecosystem(&self) -> &'static str {
        "cargo"
    }

    fn current_state(&self, root: &Path) -> Self::State {
        current_state(root)
    }

    fn requires_operator_action(&self, state: &Self::State) -> bool {
        state.requires_operator_action()
    }

    fn is_locked(&self, state: &Self::State) -> bool {
        matches!(state, LockLifecycleState::Locked { .. })
    }

    fn is_missing_lock(&self, state: &Self::State) -> bool {
        matches!(state, LockLifecycleState::MissingLock)
    }

    fn snapshot(&self, root: &Path) -> Result<(), gen_types::LockError> {
        use gen_types::LockError;
        match self.current_state(root) {
            LockLifecycleState::MissingLock => Err(LockError::MissingLockfile {
                path: root.join("Cargo.lock"),
            }),
            _ => crate::build_spec::generate_multi_target_and_write(root)
                .map(|_| ())
                .map_err(|e| LockError::SpecGeneration {
                    action: "snapshot",
                    source: Box::new(e),
                }),
        }
    }

    fn update(&self, root: &Path) -> Result<Self::Diff, gen_types::LockError> {
        use gen_types::LockError;
        let spec_path = root.join("Cargo.build-spec.json");
        let old_spec = if spec_path.exists() {
            match std::fs::read(&spec_path) {
                Ok(b) => serde_json::from_slice::<BuildSpec>(&b).ok(),
                Err(_) => None,
            }
        } else {
            None
        };
        crate::build_spec::generate_multi_target_and_write(root).map_err(|e| {
            LockError::SpecGeneration {
                action: "update",
                source: Box::new(e),
            }
        })?;
        let new_bytes =
            std::fs::read(&spec_path).map_err(|source| LockError::Io {
                action: "update",
                path: spec_path.clone(),
                source,
            })?;
        let new_spec: BuildSpec =
            serde_json::from_slice(&new_bytes).map_err(|e| LockError::SpecGeneration {
                action: "update",
                source: Box::new(e),
            })?;
        Ok(old_spec
            .as_ref()
            .map_or_else(LockDiff::default, |o| diff_specs(o, &new_spec)))
    }

    fn reset(&self, root: &Path) -> Result<(), gen_types::LockError> {
        use gen_types::LockError;
        let spec_path = root.join("Cargo.build-spec.json");
        if spec_path.exists() {
            std::fs::remove_file(&spec_path).map_err(|source| LockError::Io {
                action: "reset",
                path: spec_path,
                source,
            })?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_spec::{
        BuildSpec, CrateSource, CrateSpec, BuildRustCrateArgs, WorkspaceSpec,
    };
    use std::collections::BTreeMap;

    fn empty_spec(version: u32) -> BuildSpec {
        BuildSpec {
            version,
            workspace: WorkspaceSpec { members: vec![] },
            crates: BTreeMap::new(),
            root_crate: String::new(),
            workspace_members: vec![],
            flake_metadata: BTreeMap::new(),
            target_resolves: None,
            cargo_lock_sha256: None,
        }
    }

    fn make_crate(name: &str, version: &str, source: CrateSource) -> CrateSpec {
        CrateSpec {
            name: name.to_string(),
            version: version.to_string(),
            edition: "2024".to_string(),
            source,
            features: vec![],
            proc_macro: false,
            build_script: None,
            links: None,
            quirks: vec![],
            binaries: vec![],
            lib_target: None,
            dependencies: vec![],
            runtime_dependencies: vec![],
            build_dependencies: vec![],
            crate_renames: BTreeMap::new(),
            build_rust_crate_args: BuildRustCrateArgs::default(),
        }
    }

    fn registry(name: &str) -> CrateSource {
        CrateSource::Registry {
            url: format!("https://static.crates.io/crates/{name}/{name}-1.0.0.crate"),
            sha256: "deadbeef".to_string(),
            name_with_ext: format!("{name}-1.0.0.tar.gz"),
        }
    }

    fn git(url: &str, rev: &str) -> CrateSource {
        CrateSource::Git {
            url: url.to_string(),
            rev: rev.to_string(),
            sha256: Some("abc".to_string()),
        }
    }

    #[test]
    fn empty_diff_for_identical_specs() {
        let a = empty_spec(8);
        let b = empty_spec(8);
        let d = diff_specs(&a, &b);
        assert!(d.is_empty());
        assert_eq!(d.total_moves(), 0);
    }

    #[test]
    fn add_detection() {
        let mut a = empty_spec(8);
        let mut b = empty_spec(8);
        b.crates.insert(
            "serde-1.0.0".into(),
            make_crate("serde", "1.0.0", registry("serde")),
        );
        let _ = a;
        let d = diff_specs(&empty_spec(8), &b);
        assert_eq!(d.added.len(), 1);
        assert_eq!(d.added[0].name, "serde");
        assert_eq!(d.added[0].version, "1.0.0");
        assert_eq!(d.added[0].source_kind, "registry");
    }

    #[test]
    fn remove_detection() {
        let mut a = empty_spec(8);
        a.crates.insert(
            "serde-1.0.0".into(),
            make_crate("serde", "1.0.0", registry("serde")),
        );
        let b = empty_spec(8);
        let d = diff_specs(&a, &b);
        assert_eq!(d.removed.len(), 1);
        assert_eq!(d.removed[0].name, "serde");
    }

    #[test]
    fn upgrade_detection() {
        let mut a = empty_spec(8);
        a.crates.insert(
            "serde-1.0.0".into(),
            make_crate("serde", "1.0.0", registry("serde")),
        );
        let mut b = empty_spec(8);
        b.crates.insert(
            "serde-1.1.0".into(),
            make_crate("serde", "1.1.0", registry("serde")),
        );
        let d = diff_specs(&a, &b);
        assert_eq!(d.upgraded.len(), 1);
        assert_eq!(d.upgraded[0].name, "serde");
        assert_eq!(d.upgraded[0].from_version, "1.0.0");
        assert_eq!(d.upgraded[0].to_version, "1.1.0");
        assert!(d.added.is_empty());
        assert!(d.removed.is_empty());
    }

    #[test]
    fn source_change_detection_registry_to_git() {
        let mut a = empty_spec(8);
        a.crates.insert(
            "serde-1.0.0".into(),
            make_crate("serde", "1.0.0", registry("serde")),
        );
        let mut b = empty_spec(8);
        b.crates.insert(
            "serde-1.0.0".into(),
            make_crate(
                "serde",
                "1.0.0",
                git("https://github.com/x/y", "deadbeef"),
            ),
        );
        let d = diff_specs(&a, &b);
        assert_eq!(d.source_changed.len(), 1);
        assert_eq!(d.source_changed[0].from_kind, "registry");
        assert_eq!(d.source_changed[0].to_kind, "git");
        assert!(d.upgraded.is_empty());
    }

    #[test]
    fn schema_version_delta() {
        let a = empty_spec(7);
        let b = empty_spec(8);
        let d = diff_specs(&a, &b);
        assert_eq!(d.schema_version, Some(SchemaVersionDelta { from: 7, to: 8 }));
    }

    #[test]
    fn diff_is_deterministic_via_sorted_outputs() {
        let mut a = empty_spec(8);
        let mut b = empty_spec(8);
        for name in ["zebra", "apple", "mango"] {
            b.crates.insert(
                format!("{name}-1.0.0"),
                make_crate(name, "1.0.0", registry(name)),
            );
        }
        let _ = a.crates.insert(
            "zebra-1.0.0".into(),
            make_crate("zebra", "1.0.0", registry("zebra")),
        );
        let d = diff_specs(&a, &b);
        assert_eq!(d.added.len(), 2);
        // Sorted by name → apple before mango.
        assert_eq!(d.added[0].name, "apple");
        assert_eq!(d.added[1].name, "mango");
    }

    #[test]
    fn lifecycle_state_has_four_variants() {
        use gen_platform::TypedDispatcherTrait;
        assert_eq!(<LockLifecycleState as TypedDispatcherTrait>::variant_count(), 4);
        let kinds = <LockLifecycleState as TypedDispatcherTrait>::variant_kinds();
        assert!(kinds.contains(&"unlocked"));
        assert!(kinds.contains(&"locked"));
        assert!(kinds.contains(&"drifted"));
        assert!(kinds.contains(&"missing-lock"));
    }

    #[test]
    fn lifecycle_state_summary_matches_discriminant() {
        let cases = [
            LockLifecycleState::Unlocked { current_lock_hash: "x".into() },
            LockLifecycleState::Locked { spec_hash: "x".into(), lock_hash: "x".into() },
            LockLifecycleState::Drifted { committed_lock_hash: "x".into(), current_lock_hash: "y".into() },
            LockLifecycleState::MissingLock,
        ];
        for s in &cases {
            assert_eq!(s.summary(), s.discriminant());
        }
    }

    #[test]
    fn requires_operator_action_only_for_drift_and_missing() {
        assert!(LockLifecycleState::Drifted { committed_lock_hash: "a".into(), current_lock_hash: "b".into() }.requires_operator_action());
        assert!(LockLifecycleState::MissingLock.requires_operator_action());
        assert!(!LockLifecycleState::Unlocked { current_lock_hash: "x".into() }.requires_operator_action());
        assert!(!LockLifecycleState::Locked { spec_hash: "x".into(), lock_hash: "x".into() }.requires_operator_action());
    }
}
