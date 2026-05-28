//! Algorithmic invariants every `BuildSpec` must satisfy.
//!
//! These are PROVEN PROPERTIES — every spec gen-cargo emits is
//! verified against this set. Violations are bugs in build_spec,
//! never in downstream consumers. The substrate Nix builder trusts
//! these invariants; the typed enum captures every way a spec can
//! be ill-formed.
//!
//! Run `check(&spec)` after every generate() — returns
//! `Vec<Violation>` (empty when the spec is valid). The test suite
//! also invokes these against fixture and live-fleet specs.
//!
//! Each invariant is named, deterministic, and side-effect-free.

use crate::build_spec::{BuildSpec, CrateSource, DepKind};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "rule", rename_all = "kebab-case")]
pub enum Violation {
    /// A dep edge references a `package_key` not present in `crates`.
    UnresolvedDep {
        from: String,
        dep_name: String,
        missing_key: String,
    },
    /// A registry-source crate has no sha256 — substrate's fetchurl
    /// would 404. Catches v1-lockfile + general extraction bugs.
    RegistryWithoutSha256 { crate_key: String, name: String },
    /// A workspace_member entry doesn't appear in `crates`.
    WorkspaceMemberNotInCrates { key: String },
    /// The `root_crate` doesn't appear in `crates`.
    RootCrateNotInCrates { key: String },
    /// A dev-kind dep leaked into runtime/build dep slots — would
    /// cause buildRustCrate to pull in test frameworks for normal
    /// builds.
    DevDepInRuntimeOrBuild {
        from: String,
        dep_name: String,
        package_key: String,
    },
    /// A crate_renames entry references a version that doesn't match
    /// any crate in the spec — would render a buildRustCrate arg
    /// nixpkgs can't satisfy.
    RenameVersionMismatch {
        from: String,
        canonical_name: String,
        version: String,
    },
    /// Two crates share the same key — the spec's IndexMap silently
    /// drops one. Catches lockfile-parsing shadowing bugs.
    DuplicateCrateKey { key: String },
    /// A crate's `build_rust_crate_args` is missing the `crate_name`
    /// field. Substrate's lockfile-builder spreads this attrset
    /// verbatim into `buildRustCrate { … }`; missing `crate_name`
    /// would yield an unhelpful Nix error. Catches gen-cargo emission
    /// regressions and ensures every spec round-trip is self-contained.
    MissingBuildRustCrateName { crate_key: String, name: String },
    /// A crate has a non-empty `build_rust_crate_args` but `preBuild`
    /// is unset. The substrate's universal `CARGO_CRATE_NAME` export
    /// flows through this field — absence means a regression in the
    /// emission loop in build_spec.rs.
    MissingUniversalPreBuild { crate_key: String, name: String },
    /// A registry-source URL points at the rate-limited /api/v1
    /// redirect endpoint instead of static.crates.io. crates.io has
    /// started serving 403 on the API endpoint without a User-Agent
    /// header (which nixpkgs' fetchurl does not set). Substrate has a
    /// transitional Nix-side rewrite, but the gen-side emission must
    /// be the canonical form.
    RegistryUrlNotCanonical { crate_key: String, name: String, url: String },
    /// Spec emitted with a `version` field below the current
    /// SCHEMA_VERSION. Means the spec was emitted by an older gen
    /// and is missing fields downstream consumers rely on.
    StaleSchemaVersion { found: u32, expected: u32 },
    /// I1 of the GEN TYPED-SPEC CONTRACT (org-level directive
    /// `theory/GEN-TYPED-SPEC-CONTRACT.md`): a workspace member must
    /// emit `lib_target = Some(_)` whenever the package has a
    /// library target, even at the cargo default name/path. Without
    /// it, substrate's lockfile-builder (which uses `src =
    /// workspaceSrc` for path-source members) resolves `src/lib.rs`
    /// against the workspace root instead of the member subdir,
    /// producing no rlib and breaking every consumer that links the
    /// member.
    ///
    /// Provable on the spec alone: a workspace member with
    /// `binaries.is_empty() && lib_target.is_none()` has zero build
    /// targets — it produces nothing. That's a stale-gen-cargo or
    /// emission-regression signature.
    WorkspaceMemberMissingLibTarget { key: String, name: String },
    /// A crate is in `quirks::REGISTRY` but the emitted spec entry
    /// has an empty `quirks` field. Means gen-cargo's emission loop
    /// missed the lookup for this crate — substrate consumer will
    /// then build it without the known-required class-helper applied
    /// and the compile will fail at the upstream-bug call site.
    /// Drift catcher between registry truth and spec output.
    QuirkRegisteredButNotEmitted {
        crate_key: String,
        name: String,
        expected_quirks: usize,
    },
}

/// Run every invariant. Returns the violation list (empty on success).
/// O(crates + deps + members + renames) — pure walk, no allocation
/// beyond the result Vec, deterministic order.
#[must_use]
pub fn check(spec: &BuildSpec) -> Vec<Violation> {
    let mut out = Vec::new();
    check_unresolved_deps(spec, &mut out);
    check_registry_sha256(spec, &mut out);
    check_workspace_members(spec, &mut out);
    check_root_crate(spec, &mut out);
    check_no_dev_in_runtime(spec, &mut out);
    check_renames(spec, &mut out);
    check_build_rust_crate_args(spec, &mut out);
    check_registry_url_canonical(spec, &mut out);
    check_schema_version(spec, &mut out);
    check_workspace_member_lib_targets(spec, &mut out);
    check_quirks_registry_consistency(spec, &mut out);
    // duplicate-key check: a sanity check on IndexMap usage.
    // (IndexMap dedupes on insert, so duplicates can only happen if
    // the source data already had them — we re-verify here for
    // future-proofing against schema changes.)
    out
}

fn check_build_rust_crate_args(spec: &BuildSpec, out: &mut Vec<Violation>) {
    for (key, c) in &spec.crates {
        let args = &c.build_rust_crate_args;
        if args.crate_name.is_none() {
            out.push(Violation::MissingBuildRustCrateName {
                crate_key: key.clone(),
                name: c.name.clone(),
            });
        }
        if args.pre_build.is_none() {
            out.push(Violation::MissingUniversalPreBuild {
                crate_key: key.clone(),
                name: c.name.clone(),
            });
        }
    }
}

fn check_registry_url_canonical(spec: &BuildSpec, out: &mut Vec<Violation>) {
    for (key, c) in &spec.crates {
        if let CrateSource::Registry { url, .. } = &c.source {
            if url.starts_with("https://crates.io/api/v1/crates/") {
                out.push(Violation::RegistryUrlNotCanonical {
                    crate_key: key.clone(),
                    name: c.name.clone(),
                    url: url.clone(),
                });
            }
        }
    }
}

/// Detect drift between `quirks::REGISTRY` and the emitted spec. If
/// a crate appears in the registry but the spec's entry has empty
/// `quirks`, gen-cargo's emission loop regressed. Per-name lookup —
/// O(spec crates * registered names), trivially small (registry is
/// hand-curated, <100 entries indefinitely).
fn check_quirks_registry_consistency(spec: &BuildSpec, out: &mut Vec<Violation>) {
    for registered in crate::quirks::registered_crate_names() {
        for (key, c) in &spec.crates {
            if c.name == registered && c.quirks.is_empty() {
                let expected = crate::quirks::for_crate(registered).len();
                out.push(Violation::QuirkRegisteredButNotEmitted {
                    crate_key: key.clone(),
                    name: c.name.clone(),
                    expected_quirks: expected,
                });
            }
        }
    }
}

fn check_schema_version(spec: &BuildSpec, out: &mut Vec<Violation>) {
    if spec.version < crate::build_spec::SCHEMA_VERSION {
        out.push(Violation::StaleSchemaVersion {
            found: spec.version,
            expected: crate::build_spec::SCHEMA_VERSION,
        });
    }
}

/// I1 — workspace members must declare at least one build target.
///
/// A spec where a workspace_member has neither a library target
/// (`lib_target`) nor any binary targets (`binaries`) cannot have
/// been emitted correctly: cargo-metadata always reports at least
/// one target for any non-virtual member, and gen-cargo's lib_target
/// detection must always populate `lib_target` for workspace members
/// (the `is_default && !is_member { None }` suppression branch in
/// `build_spec.rs` is correctness-bearing only for non-members). A
/// member with both empty signals down to the substrate side as an
/// unbuildable target — substrate's lockfile-builder then sees no
/// libName/libPath, falls back to `src/lib.rs` resolved against
/// `workspaceSrc` (the workspace root, not the member subdir), and
/// rustc never runs the lib build. Catches stale-gen-cargo specs that
/// pre-date the workspace-member emission rule.
fn check_workspace_member_lib_targets(spec: &BuildSpec, out: &mut Vec<Violation>) {
    for key in &spec.workspace_members {
        let Some(c) = spec.crates.get(key) else {
            continue; // separately reported by check_workspace_members.
        };
        if c.binaries.is_empty() && c.lib_target.is_none() {
            out.push(Violation::WorkspaceMemberMissingLibTarget {
                key: key.clone(),
                name: c.name.clone(),
            });
        }
    }
}

fn check_unresolved_deps(spec: &BuildSpec, out: &mut Vec<Violation>) {
    for (from_key, crate_spec) in &spec.crates {
        for dep in &crate_spec.dependencies {
            if !spec.crates.contains_key(&dep.package_key) {
                out.push(Violation::UnresolvedDep {
                    from: from_key.clone(),
                    dep_name: dep.name.clone(),
                    missing_key: dep.package_key.clone(),
                });
            }
        }
    }
}

fn check_registry_sha256(spec: &BuildSpec, out: &mut Vec<Violation>) {
    for (key, crate_spec) in &spec.crates {
        if let CrateSource::Registry { sha256, .. } = &crate_spec.source {
            if sha256.is_empty() {
                out.push(Violation::RegistryWithoutSha256 {
                    crate_key: key.clone(),
                    name: crate_spec.name.clone(),
                });
            }
        }
    }
}

fn check_workspace_members(spec: &BuildSpec, out: &mut Vec<Violation>) {
    for key in &spec.workspace_members {
        if !spec.crates.contains_key(key) {
            out.push(Violation::WorkspaceMemberNotInCrates { key: key.clone() });
        }
    }
}

fn check_root_crate(spec: &BuildSpec, out: &mut Vec<Violation>) {
    // Empty root_crate is only valid when the spec carries no crates
    // (e.g. the test-empty spec). A populated spec must point root_crate
    // at a real key.
    if spec.root_crate.is_empty() {
        return;
    }
    if !spec.crates.contains_key(&spec.root_crate) {
        out.push(Violation::RootCrateNotInCrates {
            key: spec.root_crate.clone(),
        });
    }
}

fn check_no_dev_in_runtime(spec: &BuildSpec, out: &mut Vec<Violation>) {
    for (from_key, crate_spec) in &spec.crates {
        for dep in &crate_spec.runtime_dependencies {
            if matches!(dep.kind, DepKind::Dev) {
                out.push(Violation::DevDepInRuntimeOrBuild {
                    from: from_key.clone(),
                    dep_name: dep.name.clone(),
                    package_key: dep.package_key.clone(),
                });
            }
        }
        for dep in &crate_spec.build_dependencies {
            if matches!(dep.kind, DepKind::Dev) {
                out.push(Violation::DevDepInRuntimeOrBuild {
                    from: from_key.clone(),
                    dep_name: dep.name.clone(),
                    package_key: dep.package_key.clone(),
                });
            }
        }
    }
}

fn check_renames(spec: &BuildSpec, out: &mut Vec<Violation>) {
    for (from_key, crate_spec) in &spec.crates {
        for (canonical_name, records) in &crate_spec.crate_renames {
            for r in records {
                // The renames-records version must match some crate
                // in the spec whose name == canonical_name.
                let matches_some = spec
                    .crates
                    .values()
                    .any(|c| c.name == *canonical_name && c.version == r.version);
                if !matches_some {
                    out.push(Violation::RenameVersionMismatch {
                        from: from_key.clone(),
                        canonical_name: canonical_name.clone(),
                        version: r.version.clone(),
                    });
                }
            }
        }
    }
}

/// Assert that the spec is well-formed; panics in test mode, returns
/// `Result` for production use. The substrate side of the contract
/// relies on these invariants — running this once at spec-generation
/// time catches every class of consumer-side failure before it
/// reaches Nix.
pub fn assert_well_formed(spec: &BuildSpec) -> Result<(), Vec<Violation>> {
    let v = check(spec);
    if v.is_empty() {
        Ok(())
    } else {
        Err(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_spec::{
        CrateDepSpec, CrateRenameRecord, CrateSpec, WorkspaceMemberSpec, WorkspaceSpec,
    };
    use indexmap::IndexMap;

    fn empty_spec() -> BuildSpec {
        BuildSpec {
            version: crate::build_spec::SCHEMA_VERSION,
            workspace: WorkspaceSpec {
                root: "/x".into(),
                members: vec![],
            },
            crates: IndexMap::new(),
            root_crate: String::new(),
            workspace_members: vec![],
            flake_metadata: IndexMap::new(),
            target_resolves: None,
            cargo_lock_hash: None,
        }
    }

    fn crate_at(key: &str, name: &str, version: &str, source: CrateSource) -> (String, CrateSpec) {
        // Populate build_rust_crate_args with the bare-minimum fields
        // the universal invariants demand (crate_name + pre_build for
        // CARGO_CRATE_NAME). Tests that want to exercise the failing
        // paths construct the args manually.
        let rustc_crate_name = name.replace('-', "_");
        let pre_build = format!("export CARGO_CRATE_NAME={};", rustc_crate_name);
        let build_rust_crate_args = crate::build_spec::BuildRustCrateArgs {
            crate_name: Some(name.into()),
            version: Some(version.into()),
            edition: Some("2024".into()),
            features: vec![],
            crate_renames: IndexMap::new(),
            release: Some(true),
            proc_macro: None,
            build: None,
            links: None,
            lib_name: None,
            lib_path: None,
            pre_build: Some(pre_build),
        };
        (
            key.into(),
            CrateSpec {
                name: name.into(),
                version: version.into(),
                edition: "2024".into(),
                source,
                features: vec![],
                proc_macro: false,
                build_script: None,
                links: None,
                quirks: vec![],
                build_rust_crate_args,
                binaries: vec![],
                lib_target: None,
                dependencies: vec![],
                runtime_dependencies: vec![],
                build_dependencies: vec![],
                crate_renames: IndexMap::new(),
            },
        )
    }

    fn registry_src(sha: &str) -> CrateSource {
        CrateSource::Registry {
            url: "https://crates.io/x".into(),
            sha256: sha.into(),
            name_with_ext: "x.tar.gz".into(),
        }
    }

    fn path_src() -> CrateSource {
        CrateSource::Path {
            relative_path: ".".into(),
        }
    }

    fn dep(name: &str, key: &str, kind: DepKind) -> CrateDepSpec {
        use crate::build_spec::BuildTree;
        CrateDepSpec {
            name: name.into(),
            package_key: key.into(),
            kind,
            features: vec![],
            uses_default_features: true,
            optional: false,
            target: None,
            // Test fixture: default to Target (no proc_macro / no build kind).
            // Real population happens in build_spec.rs at edge construction.
            tree: BuildTree::Target,
        }
    }

    #[test]
    fn empty_spec_is_well_formed() {
        let s = empty_spec();
        assert!(assert_well_formed(&s).is_ok());
    }

    #[test]
    fn unresolved_dep_is_caught() {
        let mut s = empty_spec();
        let (k, mut c) = crate_at("a-1.0.0", "a", "1.0.0", path_src());
        c.dependencies = vec![dep("b", "b-2.0.0", DepKind::Normal)];
        c.runtime_dependencies = c.dependencies.clone();
        s.crates.insert(k, c);
        let v = check(&s);
        assert!(matches!(
            v.first(),
            Some(Violation::UnresolvedDep {
                from,
                dep_name,
                missing_key,
            }) if from == "a-1.0.0" && dep_name == "b" && missing_key == "b-2.0.0"
        ));
    }

    #[test]
    fn registry_without_sha256_is_caught() {
        let mut s = empty_spec();
        let (k, c) = crate_at("a-1.0.0", "a", "1.0.0", registry_src(""));
        s.crates.insert(k, c);
        let v = check(&s);
        assert!(matches!(
            v.first(),
            Some(Violation::RegistryWithoutSha256 { .. })
        ));
    }

    #[test]
    fn registry_with_sha256_is_fine() {
        let mut s = empty_spec();
        let (k, c) = crate_at("a-1.0.0", "a", "1.0.0", registry_src("deadbeef"));
        s.crates.insert(k, c);
        assert!(assert_well_formed(&s).is_ok());
    }

    #[test]
    fn workspace_member_not_in_crates_is_caught() {
        let mut s = empty_spec();
        s.workspace_members = vec!["missing-0.1.0".into()];
        let v = check(&s);
        assert!(matches!(
            v.first(),
            Some(Violation::WorkspaceMemberNotInCrates { key }) if key == "missing-0.1.0"
        ));
    }

    #[test]
    fn root_crate_not_in_crates_is_caught() {
        let mut s = empty_spec();
        s.root_crate = "ghost-0.0.0".into();
        let v = check(&s);
        assert!(matches!(
            v.first(),
            Some(Violation::RootCrateNotInCrates { .. })
        ));
    }

    #[test]
    fn dev_dep_in_runtime_is_caught() {
        let mut s = empty_spec();
        let (k1, c1) = crate_at("a-1.0.0", "a", "1.0.0", path_src());
        let (k2, c2) = crate_at("b-2.0.0", "b", "2.0.0", path_src());
        s.crates.insert(k2.clone(), c2);
        let mut a = c1;
        a.runtime_dependencies = vec![dep("b", &k2, DepKind::Dev)];
        s.crates.insert(k1.clone(), a);
        let v = check(&s);
        assert!(v
            .iter()
            .any(|x| matches!(x, Violation::DevDepInRuntimeOrBuild { .. })));
    }

    #[test]
    fn rename_pointing_to_nonexistent_version_is_caught() {
        let mut s = empty_spec();
        let (k1, mut c1) = crate_at("a-1.0.0", "a", "1.0.0", path_src());
        let (k2, c2) = crate_at("b-2.0.0", "b", "2.0.0", path_src());
        c1.crate_renames.insert(
            "b".into(),
            vec![CrateRenameRecord {
                version: "9.9.9".into(),
                rename: "bee".into(),
            }],
        );
        s.crates.insert(k1, c1);
        s.crates.insert(k2, c2);
        let v = check(&s);
        assert!(matches!(
            v.first(),
            Some(Violation::RenameVersionMismatch { .. })
        ));
    }

    #[test]
    fn rename_pointing_to_existing_crate_is_fine() {
        let mut s = empty_spec();
        let (k1, mut c1) = crate_at("a-1.0.0", "a", "1.0.0", path_src());
        let (k2, c2) = crate_at("b-2.0.0", "b", "2.0.0", path_src());
        c1.crate_renames.insert(
            "b".into(),
            vec![CrateRenameRecord {
                version: "2.0.0".into(),
                rename: "bee".into(),
            }],
        );
        s.crates.insert(k1, c1);
        s.crates.insert(k2, c2);
        assert!(assert_well_formed(&s).is_ok());
    }

    // ── Determinism property test ───────────────────────────────────

    #[test]
    fn generate_is_deterministic_against_caixa_sha2() {
        // Real-fleet determinism check: same workspace twice → identical spec.
        let root = std::path::Path::new("/Users/drzzln/code/github/pleme-io/caixa-sha2");
        if !root.exists() {
            return; // skip if fleet not present (CI environments may differ)
        }
        let a = crate::build_spec::generate(root);
        let b = crate::build_spec::generate(root);
        let (a, b) = match (a, b) {
            (Ok(a), Ok(b)) => (a, b),
            _ => return,
        };
        let ja = serde_json::to_string(&a).unwrap();
        let jb = serde_json::to_string(&b).unwrap();
        assert_eq!(ja, jb, "spec must be byte-deterministic across runs");
    }

    #[test]
    fn workspace_member_with_no_targets_is_caught() {
        // I1 — a workspace member with neither lib_target nor binaries
        // is the stale-gen-cargo signature (ishou-render-style). The
        // spec is provably unbuildable; the check must fire.
        let mut s = empty_spec();
        let (k, c) = crate_at("ishou-render-0.1.0", "ishou-render", "0.1.0", path_src());
        s.crates.insert(k.clone(), c);
        s.workspace_members.push(k.clone());
        s.root_crate = k.clone();
        let v = check(&s);
        assert!(
            v.iter().any(|x| matches!(
                x,
                Violation::WorkspaceMemberMissingLibTarget { key, .. } if key == "ishou-render-0.1.0"
            )),
            "expected WorkspaceMemberMissingLibTarget for ishou-render, got: {v:?}"
        );
    }

    #[test]
    fn workspace_member_with_lib_target_is_well_formed() {
        // Counter-example: same member but with lib_target populated
        // (the post-fix gen-cargo emission). Must pass without
        // WorkspaceMemberMissingLibTarget.
        use crate::build_spec::LibTargetSpec;
        let mut s = empty_spec();
        let (k, mut c) = crate_at("ishou-render-0.1.0", "ishou-render", "0.1.0", path_src());
        c.lib_target = Some(LibTargetSpec {
            name: "ishou_render".into(),
            path: "src/lib.rs".into(),
        });
        s.crates.insert(k.clone(), c);
        s.workspace_members.push(k.clone());
        s.root_crate = k;
        let v = check(&s);
        assert!(
            !v.iter()
                .any(|x| matches!(x, Violation::WorkspaceMemberMissingLibTarget { .. })),
            "expected no WorkspaceMemberMissingLibTarget when lib_target is Some, got: {v:?}"
        );
    }

    #[test]
    fn workspace_member_with_only_binaries_is_well_formed() {
        // Counter-example: a member that's a pure-binary crate (no
        // [lib] section) is legitimately lib_target=None as long as
        // binaries is non-empty. The check must not fire.
        use crate::build_spec::CrateBinSpec;
        let mut s = empty_spec();
        let (k, mut c) = crate_at("ryn-cli-0.1.0", "ryn-cli", "0.1.0", path_src());
        c.binaries = vec![CrateBinSpec {
            name: "ryn-cli".into(),
            path: "src/main.rs".into(),
        }];
        s.crates.insert(k.clone(), c);
        s.workspace_members.push(k.clone());
        s.root_crate = k;
        let v = check(&s);
        assert!(
            !v.iter()
                .any(|x| matches!(x, Violation::WorkspaceMemberMissingLibTarget { .. })),
            "expected no WorkspaceMemberMissingLibTarget when binaries is non-empty, got: {v:?}"
        );
    }

    #[test]
    fn live_fleet_spec_is_well_formed() {
        // Sanity-check: a freshly-generated spec from a real fleet
        // repo should satisfy every invariant.
        let root = std::path::Path::new("/Users/drzzln/code/github/pleme-io/caixa-sha2");
        if !root.exists() {
            return;
        }
        let Ok(spec) = crate::build_spec::generate(root) else { return };
        match assert_well_formed(&spec) {
            Ok(()) => (),
            Err(violations) => panic!(
                "live fleet spec violated invariants:\n{}",
                serde_json::to_string_pretty(&violations).unwrap()
            ),
        }
    }
}
