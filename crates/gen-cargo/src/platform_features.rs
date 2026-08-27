//! Typed `PlatformFeatureRegistry` — the canonical knowledge of
//! "which third-party upstream crates have platform-specific features
//! in their default-feature set that can leak across cargo's global
//! feature unification."
//!
//! The problem this module solves: Cargo features unify GLOBALLY at
//! resolve time. If a crate's `Cargo.toml` declares
//! `default = ["macos_fsevent"]` and any workspace consumer pulls
//! that crate WITHOUT `default-features = false`, the
//! `macos_fsevent` feature activates for EVERY build of that crate
//! — including linux/musl/windows targets where the feature pulls
//! `mod fsevent;` referencing `fsevent-sys` (apple-only) and the
//! cross-target compile fails.
//!
//! The substrate's `pleme-crate-overrides.notify` (triple-aware) is
//! the build-time safety net that strips apple-only features from
//! non-apple builds. This module is the gen-time **awareness layer**:
//! `diagnose(spec)` walks `target_resolves` and emits a typed
//! `Diagnostic` whenever a known platform-feature shows up in a
//! target whose tag doesn't match.
//!
//! Composes with substrate's I5: substrate corrects the leak at
//! build time; gen tells the operator their consumer Cargo.toml has
//! a leak so they can land the upstream fix (`default-features =
//! false`) and stop relying on the safety net.
//!
//! Adding a new entry is one row in `registry()`. Adding a new
//! platform is one new `PlatformTag` variant + matcher branch.

use serde::{Deserialize, Serialize};

/// Platform tag a feature is bound to. A feature with `tag = Apple`
/// is meaningful ONLY on apple targets; activating it on a
/// linux/windows target is the leak signature.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PlatformTag {
    /// `*-apple-darwin` (macOS, iOS substrate, …). Tags features that
    /// reference CoreFoundation / FSEvents / Cocoa / Metal-only paths.
    Apple,
    /// `*-unknown-linux-*` (gnu, musl, …). Tags features that reference
    /// inotify, eventfd, libudev, dbus, X11-only paths.
    Linux,
    /// `*-pc-windows-*` (msvc, gnu). Tags features that reference
    /// Win32 / NTAPI / WinRT paths.
    Windows,
}

impl PlatformTag {
    /// True if a target triple matches this platform tag. Both
    /// schema-v5 triples (`aarch64-apple-darwin`) and nixpkgs-style
    /// short names (`aarch64-darwin`) are recognized; the substrate's
    /// `pleme-crate-overrides.nix::isApple` uses the same matchers,
    /// so the gen-side warning and substrate-side strip stay aligned
    /// without a shared lookup table.
    #[must_use]
    pub fn matches_triple(self, triple: &str) -> bool {
        match self {
            Self::Apple => triple.contains("apple") || triple.contains("darwin"),
            Self::Linux => triple.contains("linux"),
            Self::Windows => triple.contains("windows"),
        }
    }
}

/// A registry entry: crate `name` has feature `feature` which is
/// only meaningful on platforms tagged `tag`.
///
/// The shape is intentionally flat — three columns, no nested
/// structure — so each addition is one self-contained row in
/// `registry()`. Future entries land here as the fleet discovers
/// more leak classes (rustls backends, mio io_uring, …).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlatformFeature {
    /// Cargo crate name (e.g. `"notify"`).
    pub crate_name: &'static str,
    /// Feature name as declared in the crate's `Cargo.toml`
    /// (e.g. `"macos_fsevent"`).
    pub feature: &'static str,
    /// Platform this feature is bound to.
    pub tag: PlatformTag,
    /// Free-form note (one line) explaining why activating this
    /// feature on a non-`tag` target breaks the cross-target build.
    /// Surfaces in the diagnostic message so the operator gets
    /// actionable context.
    pub note: &'static str,
}

/// Canonical registry. Sorted by `(crate_name, feature)` for stable
/// diffs.
///
/// Every entry documents WHY the feature is platform-bound. The
/// pleme-io substrate's `pleme-crate-overrides.nix` mirrors this
/// registry on its strip side — adding a new entry here is the
/// gen-side half; adding the matching strip rule in the substrate
/// is the build-side half. The two halves compose:
///
/// - gen: surface the leak to the operator (drive consumer-side fix)
/// - substrate: correct the leak at build time (unblock fleet builds
///   that haven't landed the upstream fix yet)
#[must_use]
pub fn registry() -> &'static [PlatformFeature] {
    &[
        // notify v8 default-features include macos_fsevent. With any
        // consumer leaving default-features on, cargo unifies the
        // feature globally → notify builds on linux/musl pull
        // `mod fsevent;` referencing fsevent-sys → E0455. Substrate
        // strips this for non-apple triples via the
        // pleme-crate-overrides notify entry.
        PlatformFeature {
            crate_name: "notify",
            feature: "fsevent-sys",
            tag: PlatformTag::Apple,
            note: "Direct alias for the macos_fsevent backend. \
                   Same cross-target failure mode as macos_fsevent.",
        },
        // kqueue is the transitive helper crate macos_kqueue pulls.
        // Surfaces under the same name in target_resolves when a
        // consumer activates the feature directly (rare but possible).
        PlatformFeature {
            crate_name: "notify",
            feature: "kqueue",
            tag: PlatformTag::Apple,
            note: "Direct alias for the macos_kqueue backend. \
                   Same cross-target failure mode as macos_kqueue.",
        },
        PlatformFeature {
            crate_name: "notify",
            feature: "macos_fsevent",
            tag: PlatformTag::Apple,
            note: "macos_fsevent pulls `mod fsevent;` which depends \
                   on fsevent-sys (apple-only). Cross-target builds \
                   fail with E0455 'link kind framework is only \
                   supported on Apple targets' unless a consumer \
                   gates with `default-features = false`.",
        },
        // macos_kqueue is the second notify backend. Its name suggests
        // BSD/portable but Cargo features unify globally per-target —
        // activating it in [dependencies] adds the kqueue crate (and
        // transitively kqueue-sys) to every target's dep graph, and
        // kqueue-sys's BSD-specific type bindings (kevent / EventFilter
        // / EventFlag) fail to compile on linux x86_64 (E0412). Same
        // leak signature as macos_fsevent; same operator-side fix:
        // cfg-gate the feature behind [target.'cfg(target_os = "macos")'].
        // Surfaced fleet-wide via shikumi 5139dd2 (May 2026) before
        // 3913d35 landed the target-gate.
        PlatformFeature {
            crate_name: "notify",
            feature: "macos_kqueue",
            tag: PlatformTag::Apple,
            note: "macos_kqueue activates the kqueue → kqueue-sys chain. \
                   kqueue-sys's BSD bindings (kevent struct, EventFilter, \
                   EventFlag types) won't compile on linux — E0412 \
                   'cannot find type'. Despite the portable-sounding name, \
                   this feature is apple-only when set in [dependencies].",
        },
    ]
}

/// Lookup the registry entry for a given (crate_name, feature) pair.
/// Returns `None` if the pair isn't tracked — used by `diagnose` to
/// avoid flagging every platform feature the fleet hasn't yet
/// classified.
#[must_use]
pub fn lookup(crate_name: &str, feature: &str) -> Option<&'static PlatformFeature> {
    registry()
        .iter()
        .find(|e| e.crate_name == crate_name && e.feature == feature)
}

/// Every crate-name with at least one registered platform feature.
/// Used by tests + tooling that want to verify substrate ↔ gen
/// alignment.
#[must_use]
pub fn registered_crate_names() -> Vec<&'static str> {
    let mut out: Vec<&'static str> = registry().iter().map(|e| e.crate_name).collect();
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apple_matches_both_v5_and_short_triples() {
        assert!(PlatformTag::Apple.matches_triple("aarch64-apple-darwin"));
        assert!(PlatformTag::Apple.matches_triple("x86_64-apple-darwin"));
        assert!(PlatformTag::Apple.matches_triple("aarch64-darwin"));
        assert!(PlatformTag::Apple.matches_triple("x86_64-darwin"));
        assert!(!PlatformTag::Apple.matches_triple("x86_64-unknown-linux-musl"));
        assert!(!PlatformTag::Apple.matches_triple("aarch64-unknown-linux-gnu"));
    }

    #[test]
    fn linux_matches_gnu_and_musl() {
        assert!(PlatformTag::Linux.matches_triple("x86_64-unknown-linux-musl"));
        assert!(PlatformTag::Linux.matches_triple("aarch64-unknown-linux-gnu"));
        assert!(!PlatformTag::Linux.matches_triple("aarch64-apple-darwin"));
        assert!(!PlatformTag::Linux.matches_triple("x86_64-pc-windows-msvc"));
    }

    #[test]
    fn windows_matches_msvc_and_gnu() {
        assert!(PlatformTag::Windows.matches_triple("x86_64-pc-windows-msvc"));
        assert!(PlatformTag::Windows.matches_triple("x86_64-pc-windows-gnu"));
        assert!(!PlatformTag::Windows.matches_triple("x86_64-unknown-linux-gnu"));
    }

    #[test]
    fn registry_is_sorted_and_deduped() {
        let r = registry();
        for window in r.windows(2) {
            let a = (window[0].crate_name, window[0].feature);
            let b = (window[1].crate_name, window[1].feature);
            assert!(a <= b, "registry not sorted: {a:?} > {b:?}");
            assert!(a != b, "registry has duplicate entry: {a:?}");
        }
    }

    #[test]
    fn notify_macos_fsevent_is_apple_tagged() {
        let e =
            lookup("notify", "macos_fsevent").expect("notify/macos_fsevent should be registered");
        assert_eq!(e.tag, PlatformTag::Apple);
        // Sanity: the canonical leak signature — feature is apple-tagged,
        // does NOT match linux/musl triples.
        assert!(!e.tag.matches_triple("x86_64-unknown-linux-musl"));
    }

    #[test]
    fn lookup_returns_none_for_unregistered() {
        assert!(lookup("serde", "default").is_none());
        assert!(lookup("notify", "no-such-feature").is_none());
    }

    #[test]
    fn registered_names_is_sorted_unique() {
        let names = registered_crate_names();
        let mut sorted = names.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(names, sorted);
    }
}
