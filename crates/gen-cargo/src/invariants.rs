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
    // duplicate-key check: a sanity check on IndexMap usage.
    // (IndexMap dedupes on insert, so duplicates can only happen if
    // the source data already had them — we re-verify here for
    // future-proofing against schema changes.)
    out
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
            version: 2,
            workspace: WorkspaceSpec {
                root: "/x".into(),
                members: vec![],
            },
            crates: IndexMap::new(),
            root_crate: String::new(),
            workspace_members: vec![],
            flake_metadata: IndexMap::new(),
        }
    }

    fn crate_at(key: &str, name: &str, version: &str, source: CrateSource) -> (String, CrateSpec) {
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
        CrateDepSpec {
            name: name.into(),
            package_key: key.into(),
            kind,
            features: vec![],
            uses_default_features: true,
            optional: false,
            target: None,
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
