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
//! deterministic output. `gen confirm` includes the diagnostics in
//! its output without elevating them to errors.

use crate::build_spec::BuildSpec;
use crate::platform_features::{lookup, PlatformTag};
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
    out
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
fn upstream_fix_hint_for(crate_name: &str, _feature: &str) -> String {
    match crate_name {
        "notify" => "In the consumer's Cargo.toml, set the bare notify dep with no \
                     macOS feature flags, then opt macOS into a backend via a \
                     target-conditional block:\n  \
                     [dependencies]\n  \
                     notify = { version = \"<v>\", default-features = false }\n\n  \
                     [target.'cfg(target_os = \"macos\")'.dependencies]\n  \
                     notify = { version = \"<v>\", default-features = false, \
                     features = [\"macos_kqueue\"] }\n\n  \
                     Linux falls back to inotify (notify's only Linux backend) \
                     with no macOS-side dep activation. The target-gate is required \
                     because Cargo features unify globally per-target — putting \
                     `features = [\"macos_kqueue\"]` in [dependencies] would still \
                     activate kqueue → kqueue-sys on every Linux build."
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
            pre_build: Some(format!("export CARGO_CRATE_NAME={};", name.replace('-', "_"))),
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
        }
    }

    #[test]
    fn fix_hint_for_notify_recommends_target_gated_macos_kqueue() {
        // The hint must steer the operator to the cfg-gated form —
        // putting `features = ["macos_kqueue"]` in plain [dependencies]
        // is the trap that produced shikumi 5139dd2's Linux-breaking
        // lockfile. The hint must call out both halves: bare notify in
        // [dependencies] AND the target-gated opt-in block.
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
                    upstream_fix_hint.contains("macos_kqueue"),
                    "hint must name a concrete macOS backend (macos_kqueue), got: {upstream_fix_hint}"
                );
                assert!(
                    upstream_fix_hint.contains("inotify"),
                    "hint must explain Linux falls back to inotify, got: {upstream_fix_hint}"
                );
            }
        }
    }
}
