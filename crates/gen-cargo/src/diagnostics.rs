//! Typed gen-cargo **diagnostics** — non-fatal observations about a
//! spec that the operator should address even though the spec is
//! still well-formed cargo-wise.
//!
//! Diagnostics differ from [`crate::invariants::Violation`] in
//! semantic level:
//!
//! - **Violation** = the spec is provably wrong; substrate cannot
//!   consume it correctly. `assert_well_formed` panics / errors.
//! - **Diagnostic** = the spec is cargo-correct but carries a known
//!   leak/risk class that substrate has a safety net for. The
//!   operator gets a typed warning naming the upstream fix
//!   (consumer Cargo.toml change) so the fleet can shed the safety
//!   net's dependency over time.
//!
//! The first variant (`PlatformFeatureLeakAcrossTargets`) captures
//! the cargo-feature-unification class — discovered fleet-wide when
//! the notify v8 default-feature `macos_fsevent` leaked onto
//! linux/musl builds. Substrate's `pleme-crate-overrides.notify`
//! (triple-aware) corrects the build; this diagnostic surfaces the
//! consumer-side fix so the operator can land it.
//!
//! Run `diagnose(&spec)` after `generate_multi_target` — returns
//! `Vec<Diagnostic>` (empty when no leaks). Pure walk, no I/O,
//! deterministic output.
//!
//! ★ Corrected 2026-07-31. This doc used to claim "`gen confirm` includes the
//! diagnostics in its output without elevating them to errors." That was
//! false, and load-bearingly so: `gen confirm` is a pure OFFLINE
//! delta-freshness check that reads only `Cargo.lock` + `Cargo.gen.lock`, has
//! no `BuildSpec` in scope, and cannot call `diagnose` as written. An audit
//! found `diagnose` had ZERO production call sites — the type was referenced
//! nowhere outside this module and its own tests. The whole surface was
//! declared and unreferenced while its doc asserted it was wired, which is
//! strictly worse than an honest TODO: it reads as covered.
//!
//! It is now emitted from `build_spec::generate_multi_target_and_write`, the
//! default `gen build` path, as advisory stderr lines. Still non-fatal by
//! design — elevating it to an exit code is a separate, deliberate decision
//! that should follow observing it fire on real specs first.

use crate::build_spec::BuildSpec;
use crate::platform_features::{PlatformTag, lookup};
use serde::{Deserialize, Serialize};

/// Typed diagnostic emitted by `diagnose`. Variants are tagged
/// (`#[serde(tag = "rule")]`) so consumers (gen confirm, cse-lint,
/// IDE tooling) can dispatch on the variant string without
/// re-implementing the matcher.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "rule", rename_all = "kebab-case")]
pub enum Diagnostic {
    /// Cargo's global feature unification activated a
    /// platform-specific feature on a target whose platform tag
    /// doesn't match. Symptom: the cross-target build fails unless
    /// the substrate safety net (triple-aware override) strips the
    /// feature for that target.
    ///
    /// Operator-side fix: in the consumer's `Cargo.toml`, set
    /// `default-features = false` on the leaking crate dependency
    /// and re-add only the cross-platform features it actually
    /// needs, or move the platform-feature into a
    /// `[target.'cfg(target_vendor = "X")'.dependencies]` block.
    ///
    /// Example: `notify`'s `default = ["macos_fsevent"]` leaks the
    /// apple-only fsevent backend onto every linux/musl build of
    /// notify in the workspace's resolve graph. The substrate
    /// strips it; this diagnostic surfaces the leak.
    PlatformFeatureLeakAcrossTargets {
        crate_key: String,
        name: String,
        feature: String,
        feature_platform: PlatformTag,
        triples: Vec<String>,
        upstream_fix_hint: String,
    },

    /// A feature that is on the RIGHT platform but selects a strictly
    /// worse implementation than the one available beside it.
    ///
    /// This is a different failure class from the leak above and the
    /// reason it exists: a leak breaks the build, so it is discovered
    /// immediately. A degraded backend **builds and runs perfectly** —
    /// it just costs orders of magnitude more of some resource, silently,
    /// forever. Nothing fails, so nothing surfaces it.
    ///
    /// Founding instance (2026-07-31): `notify`'s `macos_kqueue`. On
    /// macOS notify picks its backend as
    /// `#[cfg(all(target_os = "macos", not(feature = "macos_kqueue")))] →
    /// FsEventWatcher`, so the feature is *poison*: Cargo features are
    /// additive and unify across the graph, so ONE crate enabling it flips
    /// EVERY macOS consumer to kqueue, and a consumer cannot un-enable it.
    /// kqueue holds one open file descriptor per WATCHED PATH; FSEvents
    /// holds none. Measured: the seki prompt daemon, watching one repo
    /// recursively, held 26,517 open FDs across 3,014 paths. Seven repos
    /// carried the declaration — shikumi first, then six more by copy,
    /// because gen's own fix-hint recommended it.
    DegradedBackendFeatureSelected {
        crate_key: String,
        name: String,
        feature: String,
        prefer: String,
        triples: Vec<String>,
        cost: String,
        upstream_fix_hint: String,
    },
}

/// Features that are platform-correct but select a strictly worse
/// implementation than a sibling feature of the same crate.
///
/// `(crate, poison_feature, prefer_feature, cost)`. Deliberately a table
/// and not an `if name == "notify"`: the class is "a backend feature you
/// can turn on but not off, whose cost is invisible at build time", and
/// notify is simply the first member we paid for. A new member is one row.
const DEGRADED_BACKENDS: &[(&str, &str, &str, &str)] = &[(
    "notify",
    "macos_kqueue",
    "macos_fsevent",
    "kqueue holds one open file descriptor per watched path; FSEvents holds \
     none. A recursive watch of one repo measured 26,517 open FDs. The feature \
     is additive and cannot be un-enabled by a consumer, so a single crate \
     enabling it degrades every macOS consumer of notify in the graph.",
)];

impl Diagnostic {
    /// One-line operator-facing summary, for emitting during a build.
    ///
    /// Deliberately names the concrete cost rather than the rule that fired.
    /// "notify activates macos_kqueue" tells a reader nothing; "one open file
    /// descriptor per watched path" tells them why they should care enough to
    /// go look.
    #[must_use]
    pub fn summary(&self) -> String {
        match self {
            Self::PlatformFeatureLeakAcrossTargets {
                name,
                feature,
                triples,
                ..
            } => format!(
                "{name}: platform feature `{feature}` leaked onto {} non-matching target(s): {}",
                triples.len(),
                triples.join(", ")
            ),
            Self::DegradedBackendFeatureSelected {
                name,
                feature,
                prefer,
                triples,
                cost,
                ..
            } => format!(
                "{name}: `{feature}` is active on {} — prefer `{prefer}`. {cost}",
                triples.join(", ")
            ),
        }
    }
}

/// Run every diagnostic against the spec. Empty `Vec` when no
/// diagnostics fire. Pure function — same inputs always produce the
/// same output, no logging, no I/O.
///
/// O(target_resolves × crates × features × registry) — registry is
/// tiny (~10s of entries indefinitely), target_resolves is bounded
/// by the FLEET_TARGETS list (currently 6), so the cost is
/// dominated by spec.crates × spec.crates.features which is what
/// every other invariant already walks.
#[must_use]
pub fn diagnose(spec: &BuildSpec) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    diagnose_platform_feature_leaks(spec, &mut out);
    diagnose_degraded_backends(spec, &mut out);
    out
}

/// Flag every (crate, feature, triple) where a [`DEGRADED_BACKENDS`] row is
/// active **on the platform it belongs to**.
///
/// The two walks partition the space rather than overlap: a platform feature
/// on the WRONG triple is a leak (it breaks the build, and
/// `diagnose_platform_feature_leaks` already reports it); the same feature on
/// its RIGHT triple is this diagnostic. That split is deliberate — the
/// right-platform case is the dangerous one precisely because it builds
/// clean and only shows up as runtime cost nobody is measuring.
fn diagnose_degraded_backends(spec: &BuildSpec, out: &mut Vec<Diagnostic>) {
    let Some(compact) = spec.target_resolves.as_ref() else {
        return;
    };
    let target_resolves = compact.expand();

    let mut grouped: indexmap::IndexMap<(String, String, String, String, String), Vec<String>> =
        indexmap::IndexMap::new();

    for (triple, resolve) in &target_resolves {
        for (key, edges) in &resolve.crates {
            let Some(crate_spec) = spec.crates.get(key) else {
                continue;
            };
            for feature in &edges.features {
                let Some((_, _, prefer, cost)) = DEGRADED_BACKENDS
                    .iter()
                    .find(|(c, f, _, _)| *c == crate_spec.name && f == feature)
                else {
                    continue;
                };
                // Only on the platform the feature belongs to — the
                // wrong-platform case is a leak and is reported there.
                if !lookup(&crate_spec.name, feature)
                    .is_some_and(|entry| entry.tag.matches_triple(triple))
                {
                    continue;
                }
                grouped
                    .entry((
                        key.clone(),
                        crate_spec.name.clone(),
                        feature.clone(),
                        (*prefer).to_string(),
                        (*cost).to_string(),
                    ))
                    .or_default()
                    .push(triple.clone());
            }
        }
    }

    for ((crate_key, name, feature, prefer, cost), mut triples) in grouped {
        triples.sort();
        triples.dedup();
        let upstream_fix_hint = upstream_fix_hint_for(&name, &feature);
        out.push(Diagnostic::DegradedBackendFeatureSelected {
            crate_key,
            name,
            feature,
            prefer,
            triples,
            cost,
            upstream_fix_hint,
        });
    }
}

/// Walk `target_resolves[triple].crates[key].features` and flag any
/// (crate, feature, triple) triple where the registered platform tag
/// doesn't match the triple. Returns one diagnostic per (crate,
/// feature) leak — `triples` collects every offending target so the
/// operator sees the full leak footprint, not one diagnostic per
/// (crate, feature, triple) row.
fn diagnose_platform_feature_leaks(spec: &BuildSpec, out: &mut Vec<Diagnostic>) {
    let Some(compact) = spec.target_resolves.as_ref() else {
        // Old-shape spec without per-target resolves — no
        // diagnostic surface to inspect.
        return;
    };
    // Expand the compact (base + overrides) form back to the full
    // per-target crates map so the leak walk sees each triple's
    // complete effective edge set (`base // overrides[triple]`).
    let target_resolves = compact.expand();

    // Group leaks: (crate_key, feature) -> Vec<triple>.
    // Use indexmap to keep deterministic order (registry-then-triple).
    let mut grouped: indexmap::IndexMap<(String, String, String, PlatformTag), Vec<String>> =
        indexmap::IndexMap::new();

    for (triple, resolve) in &target_resolves {
        for (key, edges) in &resolve.crates {
            let Some(crate_spec) = spec.crates.get(key) else {
                // Spec consistency error — would be caught by
                // invariants::check_unresolved_deps. Skip silently
                // here; not a diagnostic concern.
                continue;
            };
            for feature in &edges.features {
                let Some(entry) = lookup(&crate_spec.name, feature) else {
                    continue;
                };
                if entry.tag.matches_triple(triple) {
                    continue;
                }
                grouped
                    .entry((
                        key.clone(),
                        crate_spec.name.clone(),
                        feature.clone(),
                        entry.tag,
                    ))
                    .or_default()
                    .push(triple.clone());
            }
        }
    }

    for ((crate_key, name, feature, feature_platform), mut triples) in grouped {
        triples.sort();
        triples.dedup();
        let upstream_fix_hint = upstream_fix_hint_for(&name, &feature);
        out.push(Diagnostic::PlatformFeatureLeakAcrossTargets {
            crate_key,
            name,
            feature,
            feature_platform,
            triples,
            upstream_fix_hint,
        });
    }
}

/// Per-leak operator hint. Crate-specific where we know the exact
/// drop-in replacement; falls back to a generic
/// `default-features = false` + target-gate message otherwise.
///
/// Cargo features unify globally per-target — there is NO portable
/// macOS backend feature for a crate like notify; the only
/// kqueue-sys-free Linux build is one where the macos_* feature is
/// gated behind `[target.'cfg(target_os = "macos")']`. The hint
/// previously recommended `features = ["macos_kqueue"]` claiming it
/// was portable; that advice produced shikumi 5139dd2's Linux-breaking
/// lockfile and was corrected to the target-gated form in 3913d35.
///
/// ★ Corrected again 2026-07-31, and this one was the expensive mistake:
/// the target-gated form was right, but the FEATURE NAMED IN IT WAS NOT.
/// The hint said `macos_kqueue`, and seven repos adopted it verbatim —
/// shikumi first, then six more by copy. On macOS notify selects its
/// backend as:
///
/// ```ignore
/// #[cfg(all(target_os = "macos", not(feature = "macos_kqueue")))]
/// pub type RecommendedWatcher = FsEventWatcher;
/// ```
///
/// so `macos_kqueue` is a POISON feature: additive unification means one
/// crate enabling it flips EVERY macOS consumer to the kqueue backend, and
/// a consumer cannot un-enable it. kqueue holds one open file descriptor
/// per watched path; FSEvents holds none. Measured on cid: one prompt
/// daemon watching a single repo recursively held 26,517 open FDs.
///
/// `macos_fsevent` is the correct backend AND is Linux-safe for exactly the
/// same reason the target-gate exists: notify declares `fsevent-sys` under
/// `[target.'cfg(target_os="macos")']`, so it is unreachable off macOS. The
/// Linux breakage that motivated kqueue was never a property of fsevent.
fn upstream_fix_hint_for(crate_name: &str, _feature: &str) -> String {
    match crate_name {
        "notify" => "In the consumer's Cargo.toml, set the bare notify dep with no \
                     macOS feature flags, then opt macOS into a backend via a \
                     target-conditional block:\n  \
                     [dependencies]\n  \
                     notify = { version = \"<v>\", default-features = false }\n\n  \
                     [target.'cfg(target_os = \"macos\")'.dependencies]\n  \
                     notify = { version = \"<v>\", default-features = false, \
                     features = [\"macos_fsevent\"] }\n\n  \
                     Linux falls back to inotify (notify's only Linux backend) \
                     with no macOS-side dep activation. The target-gate is required \
                     because Cargo features unify globally per-target — putting a \
                     macOS backend feature in [dependencies] activates its dep \
                     chain on every Linux build.\n\n  \
                     Use macos_fsevent, NOT macos_kqueue. `macos_kqueue` is a \
                     poison feature: notify picks FsEventWatcher only under \
                     `not(feature = \"macos_kqueue\")`, so ONE crate enabling it \
                     flips every macOS consumer to kqueue — which holds one open \
                     file descriptor per WATCHED PATH (measured: 26,517 FDs in a \
                     single daemon) where FSEvents holds none. Features are \
                     additive; a consumer cannot un-enable it."
            .to_string(),
        _ => format!(
            "In the consumer's Cargo.toml, set `default-features = false` on \
             {crate_name}, then re-add only the cross-platform features the \
             consumer actually uses. Platform-specific features (apple-only / \
             linux-only / windows-only) must live in a \
             `[target.'cfg(target_os = \"X\")'.dependencies]` block — putting \
             them in plain [dependencies] activates them globally per-target \
             due to Cargo's feature unification, which is the leak this \
             diagnostic detected."
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_spec::{
        BuildRustCrateArgs, CompactTargetResolves, CrateSource, CrateSpec, CrateTargetEdges,
        TargetResolve, WorkspaceSpec,
    };
    use indexmap::IndexMap;
    use std::collections::BTreeMap;

    fn empty_spec() -> BuildSpec {
        BuildSpec {
            version: crate::build_spec::SCHEMA_VERSION,
            workspace: WorkspaceSpec { members: vec![] },
            crates: BTreeMap::new(),
            root_crate: String::new(),
            workspace_members: vec![],
            flake_metadata: BTreeMap::new(),
            target_resolves: None,
            cargo_lock_sha256: None,
            manifest_sha256: crate::manifest_tie::ManifestDigests::new(),
        }
    }

    fn registry_crate(name: &str, version: &str, features: Vec<String>) -> (String, CrateSpec) {
        let key = format!("{name}-{version}");
        let args = BuildRustCrateArgs {
            crate_name: Some(name.into()),
            version: Some(version.into()),
            edition: Some("2024".into()),
            features: features.clone(),
            crate_renames: BTreeMap::new(),
            release: Some(true),
            proc_macro: None,
            build: None,
            links: None,
            lib_name: None,
            lib_path: None,
            pre_build: Some(format!(
                "export CARGO_CRATE_NAME={};",
                name.replace('-', "_")
            )),
        };
        (
            key,
            CrateSpec {
                name: name.into(),
                version: version.into(),
                edition: "2024".into(),
                source: CrateSource::Registry {
                    url: "https://crates.io/x".into(),
                    sha256: "deadbeef".into(),
                    name_with_ext: format!("{name}.tar.gz"),
                },
                features,
                proc_macro: false,
                build_script: None,
                links: None,
                quirks: vec![],
                build_rust_crate_args: args,
                binaries: vec![],
                lib_target: None,
                dependencies: vec![],
                runtime_dependencies: vec![],
                build_dependencies: vec![],
                crate_renames: BTreeMap::new(),
            },
        )
    }

    fn target_resolve_for(key: &str, features: Vec<String>) -> TargetResolve {
        let mut crates = BTreeMap::new();
        crates.insert(
            key.to_string(),
            CrateTargetEdges {
                dependencies: vec![],
                runtime_dependencies: vec![],
                build_dependencies: vec![],
                features,
            },
        );
        TargetResolve { crates }
    }

    #[test]
    fn empty_spec_yields_no_diagnostics() {
        let s = empty_spec();
        assert!(diagnose(&s).is_empty());
    }

    #[test]
    fn old_shape_spec_without_target_resolves_is_skipped() {
        let mut s = empty_spec();
        let (k, c) = registry_crate("notify", "8.2.0", vec!["macos_fsevent".into()]);
        s.crates.insert(k, c);
        // target_resolves is None — no leak surface to inspect.
        assert!(diagnose(&s).is_empty());
    }

    #[test]
    fn notify_macos_fsevent_on_linux_musl_triggers_diagnostic() {
        let mut s = empty_spec();
        let (k, c) = registry_crate(
            "notify",
            "8.2.0",
            vec!["default".into(), "macos_fsevent".into()],
        );
        s.crates.insert(k.clone(), c);
        let mut tr = IndexMap::new();
        tr.insert(
            "x86_64-unknown-linux-musl".to_string(),
            target_resolve_for(&k, vec!["default".into(), "macos_fsevent".into()]),
        );
        tr.insert(
            "aarch64-apple-darwin".to_string(),
            target_resolve_for(&k, vec!["default".into(), "macos_fsevent".into()]),
        );
        s.target_resolves = Some(CompactTargetResolves::from_full(tr));
        let d = diagnose(&s);
        assert_eq!(d.len(), 1, "expected exactly one leak diagnostic");
        match &d[0] {
            Diagnostic::PlatformFeatureLeakAcrossTargets {
                name,
                feature,
                feature_platform,
                triples,
                ..
            } => {
                assert_eq!(name, "notify");
                assert_eq!(feature, "macos_fsevent");
                assert_eq!(*feature_platform, PlatformTag::Apple);
                assert_eq!(triples, &vec!["x86_64-unknown-linux-musl".to_string()]);
            }
            other => panic!("expected a leak diagnostic, got {other:?}"),
        }
    }

    #[test]
    fn apple_targets_alone_yield_no_diagnostic() {
        // The feature is apple-tagged; if it only appears on apple
        // triples, that's the correct case and no diagnostic fires.
        let mut s = empty_spec();
        let (k, c) = registry_crate("notify", "8.2.0", vec!["macos_fsevent".into()]);
        s.crates.insert(k.clone(), c);
        let mut tr = IndexMap::new();
        tr.insert(
            "aarch64-apple-darwin".to_string(),
            target_resolve_for(&k, vec!["macos_fsevent".into()]),
        );
        tr.insert(
            "x86_64-apple-darwin".to_string(),
            target_resolve_for(&k, vec!["macos_fsevent".into()]),
        );
        s.target_resolves = Some(CompactTargetResolves::from_full(tr));
        assert!(diagnose(&s).is_empty());
    }

    #[test]
    fn linux_only_with_macos_kqueue_fires_leak_diagnostic() {
        // Even with NO apple target in the resolve graph, activating
        // macos_kqueue on linux is itself the leak — the feature pulls
        // kqueue → kqueue-sys, and kqueue-sys's BSD bindings won't
        // compile on linux. Registered as Apple-tagged so the
        // diagnostic fires regardless of whether apple appears.
        let mut s = empty_spec();
        let (k, c) = registry_crate("notify", "8.2.0", vec!["macos_kqueue".into()]);
        s.crates.insert(k.clone(), c);
        let mut tr = IndexMap::new();
        tr.insert(
            "x86_64-unknown-linux-musl".to_string(),
            target_resolve_for(&k, vec!["macos_kqueue".into()]),
        );
        s.target_resolves = Some(CompactTargetResolves::from_full(tr));
        let d = diagnose(&s);
        assert_eq!(d.len(), 1, "macos_kqueue on linux must flag");
        match &d[0] {
            Diagnostic::PlatformFeatureLeakAcrossTargets {
                name,
                feature,
                feature_platform,
                triples,
                ..
            } => {
                assert_eq!(name, "notify");
                assert_eq!(feature, "macos_kqueue");
                assert_eq!(*feature_platform, PlatformTag::Apple);
                assert_eq!(triples, &vec!["x86_64-unknown-linux-musl".to_string()]);
            }
            other => panic!("expected a leak diagnostic, got {other:?}"),
        }
    }

    #[test]
    fn linux_only_with_unregistered_feature_yields_no_diagnostic() {
        // Unregistered features (no platform_features.rs entry) skip the
        // check — `lookup` returns None and the diagnostic walker
        // continues. Preserves the "don't flag features the fleet
        // hasn't classified yet" invariant.
        let mut s = empty_spec();
        let (k, c) = registry_crate("notify", "8.2.0", vec!["unregistered-feature".into()]);
        s.crates.insert(k.clone(), c);
        let mut tr = IndexMap::new();
        tr.insert(
            "x86_64-unknown-linux-musl".to_string(),
            target_resolve_for(&k, vec!["unregistered-feature".into()]),
        );
        s.target_resolves = Some(CompactTargetResolves::from_full(tr));
        assert!(diagnose(&s).is_empty());
    }

    #[test]
    fn leak_collects_every_offending_triple() {
        // Same feature leaks onto multiple non-apple triples — one
        // diagnostic should aggregate them all.
        let mut s = empty_spec();
        let (k, c) = registry_crate("notify", "8.2.0", vec!["macos_fsevent".into()]);
        s.crates.insert(k.clone(), c);
        let mut tr = IndexMap::new();
        tr.insert(
            "x86_64-unknown-linux-musl".to_string(),
            target_resolve_for(&k, vec!["macos_fsevent".into()]),
        );
        tr.insert(
            "aarch64-unknown-linux-gnu".to_string(),
            target_resolve_for(&k, vec!["macos_fsevent".into()]),
        );
        tr.insert(
            "aarch64-apple-darwin".to_string(),
            target_resolve_for(&k, vec!["macos_fsevent".into()]),
        );
        s.target_resolves = Some(CompactTargetResolves::from_full(tr));
        let d = diagnose(&s);
        assert_eq!(d.len(), 1);
        match &d[0] {
            Diagnostic::PlatformFeatureLeakAcrossTargets { triples, .. } => {
                assert_eq!(
                    triples,
                    &vec![
                        "aarch64-unknown-linux-gnu".to_string(),
                        "x86_64-unknown-linux-musl".to_string(),
                    ]
                );
            }
            other => panic!("expected a leak diagnostic, got {other:?}"),
        }
    }

    #[test]
    fn fix_hint_for_notify_recommends_target_gated_macos_fsevent() {
        // The hint must steer the operator to the cfg-gated form —
        // putting a macOS backend feature in plain [dependencies] is the
        // trap that produced shikumi 5139dd2's Linux-breaking lockfile.
        // The hint must call out both halves: bare notify in
        // [dependencies] AND the target-gated opt-in block.
        //
        // ★ And the feature it names must be macos_fsevent. Until
        // 2026-07-31 this hint recommended macos_kqueue and seven repos
        // adopted it, putting every macOS notify consumer on a backend
        // that holds one open FD per watched path (26,517 in one daemon).
        //
        // Note the assertions below check the RECOMMENDED BLOCK
        // (`features = ["..."]`), not a bare substring. The hint legitimately
        // mentions macos_kqueue when warning against it, so a
        // `contains("macos_kqueue")` assertion — which is what this test used
        // to make — passes for both the right and the wrong hint and would
        // not have caught the defect it was supposedly guarding.
        let mut s = empty_spec();
        let (k, c) = registry_crate("notify", "8.2.0", vec!["macos_fsevent".into()]);
        s.crates.insert(k.clone(), c);
        let mut tr = IndexMap::new();
        tr.insert(
            "x86_64-unknown-linux-musl".to_string(),
            target_resolve_for(&k, vec!["macos_fsevent".into()]),
        );
        s.target_resolves = Some(CompactTargetResolves::from_full(tr));
        let d = diagnose(&s);
        match &d[0] {
            Diagnostic::PlatformFeatureLeakAcrossTargets {
                upstream_fix_hint, ..
            } => {
                assert!(
                    upstream_fix_hint.contains("default-features = false"),
                    "hint must name `default-features = false`, got: {upstream_fix_hint}"
                );
                assert!(
                    upstream_fix_hint.contains("target_os = \"macos\""),
                    "hint must steer to the cfg(target_os = \"macos\") target-gate, got: {upstream_fix_hint}"
                );
                assert!(
                    upstream_fix_hint.contains("features = [\"macos_fsevent\"]"),
                    "hint must RECOMMEND the macos_fsevent backend block, got: {upstream_fix_hint}"
                );
                assert!(
                    !upstream_fix_hint.contains("features = [\"macos_kqueue\"]"),
                    "hint must NEVER recommend a macos_kqueue block — it is a poison \
                     feature that puts every macOS notify consumer on one open FD per \
                     watched path, got: {upstream_fix_hint}"
                );
                assert!(
                    upstream_fix_hint.contains("inotify"),
                    "hint must explain Linux falls back to inotify, got: {upstream_fix_hint}"
                );
            }
            other => panic!("expected a leak diagnostic, got {other:?}"),
        }
    }

    #[test]
    fn macos_kqueue_on_an_apple_triple_fires_the_degraded_backend_diagnostic() {
        // The case that cost us: macos_kqueue on its CORRECT platform. The
        // leak walk stays silent (nothing is mis-targeted), the build is
        // clean, and every macOS consumer of notify quietly moves to a
        // backend holding one FD per watched path. Nothing was watching for
        // it, which is why it ran for as long as it did.
        let mut s = empty_spec();
        let (k, c) = registry_crate("notify", "8.2.0", vec!["macos_kqueue".into()]);
        s.crates.insert(k.clone(), c);
        let mut tr = IndexMap::new();
        tr.insert(
            "aarch64-apple-darwin".to_string(),
            target_resolve_for(&k, vec!["macos_kqueue".into()]),
        );
        s.target_resolves = Some(CompactTargetResolves::from_full(tr));
        let d = diagnose(&s);
        assert_eq!(d.len(), 1, "degraded backend on apple must flag");
        match &d[0] {
            Diagnostic::DegradedBackendFeatureSelected {
                name,
                feature,
                prefer,
                triples,
                ..
            } => {
                assert_eq!(name, "notify");
                assert_eq!(feature, "macos_kqueue");
                assert_eq!(prefer, "macos_fsevent");
                assert_eq!(triples, &vec!["aarch64-apple-darwin".to_string()]);
            }
            other => panic!("expected a degraded-backend diagnostic, got {other:?}"),
        }
    }

    #[test]
    fn preferred_backend_on_an_apple_triple_is_silent() {
        // The fixed state must produce NO diagnostic — otherwise the new
        // rule is noise the fleet learns to scroll past.
        let mut s = empty_spec();
        let (k, c) = registry_crate("notify", "8.2.0", vec!["macos_fsevent".into()]);
        s.crates.insert(k.clone(), c);
        let mut tr = IndexMap::new();
        tr.insert(
            "aarch64-apple-darwin".to_string(),
            target_resolve_for(&k, vec!["macos_fsevent".into()]),
        );
        s.target_resolves = Some(CompactTargetResolves::from_full(tr));
        assert!(
            diagnose(&s).is_empty(),
            "macos_fsevent on apple is the correct state and must be silent"
        );
    }
}
