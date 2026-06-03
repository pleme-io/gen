//! Invariants over the gomod `BuildSpec`. Implements
//! `gen_types::Invariants` so cse-lint + gen confirm can call into the
//! adapter uniformly.

use serde::{Deserialize, Serialize};

use crate::build_spec::BuildSpec;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "rule", rename_all = "kebab-case")]
pub enum Violation {
    /// The spec's schema version predates the current one — regenerate.
    StaleSchemaVersion { found: u32, expected: u32 },
    /// A package has external `require` edges (`has_external_deps`) but
    /// carries no `vendorHash`. nixpkgs `buildGoModule` would `throw`
    /// "vendorHash is missing" — the build can't proceed. The only valid
    /// hash-less state is a leaf module (no external deps).
    VendorHashMissing { package: String },
    /// A package entry has an empty module path. Every Go package's
    /// `name` must be its import path; an empty one means the go.mod's
    /// `module` directive was missing or unparsed.
    EmptyModulePath { package: String },
    /// The spec's `root_package` does not key into `packages`. The
    /// substrate consumer indexes `packages[root_package]` to find the
    /// buildable package; a dangling root yields no build.
    RootPackageNotFound { root: String },
    /// A `workspace_members` entry has no corresponding package by
    /// module path. Member list and package set must agree.
    OrphanWorkspaceMember { member: String },
}

#[must_use]
pub fn check(spec: &BuildSpec) -> Vec<Violation> {
    let mut out = Vec::new();

    if spec.version < crate::build_spec::SCHEMA_VERSION {
        out.push(Violation::StaleSchemaVersion {
            found: spec.version,
            expected: crate::build_spec::SCHEMA_VERSION,
        });
    }

    // root_package must key into packages — but only meaningfully when
    // the spec HAS packages. An entirely empty spec (no packages) is a
    // degenerate but valid intermediate shape (e.g. a freshly-scaffolded
    // crate's smoke fixture); a dangling root only matters once there's
    // a package set to index into.
    if !spec.packages.is_empty() && !spec.packages.contains_key(&spec.root_package) {
        out.push(Violation::RootPackageNotFound {
            root: spec.root_package.clone(),
        });
    }

    for (key, pkg) in &spec.packages {
        // Every package needs a non-empty module path.
        if pkg.name.trim().is_empty() {
            out.push(Violation::EmptyModulePath {
                package: key.clone(),
            });
        }
        // A package with external deps MUST carry a vendorHash. A
        // hash-less package is only valid when it has no external deps
        // (leaf module, vendorHash = null upstream).
        if pkg.has_external_deps && pkg.args.vendor_hash.is_none() {
            out.push(Violation::VendorHashMissing {
                package: key.clone(),
            });
        }
    }

    // Every declared workspace member must resolve to a package by its
    // module path (the package's `name`).
    for member in &spec.workspace_members {
        let found = spec.packages.values().any(|p| &p.name == member);
        if !found {
            out.push(Violation::OrphanWorkspaceMember {
                member: member.clone(),
            });
        }
    }

    out
}

pub struct GomodInvariants;

impl gen_types::Invariants for GomodInvariants {
    type Spec = BuildSpec;
    type Violation = Violation;
    fn check(spec: &Self::Spec) -> Vec<Self::Violation> {
        check(spec)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_spec::{PackageArgs, PackageSpec};
    use indexmap::IndexMap;

    fn ok_spec() -> BuildSpec {
        let mut packages = IndexMap::new();
        packages.insert(
            "widget".to_string(),
            PackageSpec {
                name: "github.com/example/widget".to_string(),
                version: "0.0.0".to_string(),
                args: PackageArgs {
                    pname: Some("widget".into()),
                    version: Some("0.0.0".into()),
                    vendor_hash: Some("sha256-AAAA=".into()),
                    ..Default::default()
                },
                has_external_deps: true,
                quirks: Vec::new(),
            },
        );
        BuildSpec {
            version: crate::build_spec::SCHEMA_VERSION,
            packages,
            root_package: "widget".to_string(),
            workspace_members: vec!["github.com/example/widget".to_string()],
        }
    }

    #[test]
    fn well_formed_spec_has_no_violations() {
        assert!(check(&ok_spec()).is_empty());
    }

    #[test]
    fn detects_missing_vendor_hash() {
        let mut s = ok_spec();
        s.packages.get_mut("widget").unwrap().args.vendor_hash = None;
        let v = check(&s);
        assert!(v
            .iter()
            .any(|x| matches!(x, Violation::VendorHashMissing { .. })));
    }

    #[test]
    fn leaf_module_without_external_deps_is_clean() {
        // No external deps, no vendor_hash — a dependency-free module.
        let mut s = ok_spec();
        let pkg = s.packages.get_mut("widget").unwrap();
        pkg.has_external_deps = false;
        pkg.args.vendor_hash = None;
        assert!(check(&s).is_empty());
    }

    #[test]
    fn detects_dangling_root_package() {
        let mut s = ok_spec();
        s.root_package = "nonexistent".to_string();
        let v = check(&s);
        assert!(v
            .iter()
            .any(|x| matches!(x, Violation::RootPackageNotFound { .. })));
    }

    #[test]
    fn detects_orphan_workspace_member() {
        let mut s = ok_spec();
        s.workspace_members.push("github.com/example/ghost".into());
        let v = check(&s);
        assert!(v
            .iter()
            .any(|x| matches!(x, Violation::OrphanWorkspaceMember { .. })));
    }

    #[test]
    fn detects_empty_module_path() {
        let mut s = ok_spec();
        s.packages.get_mut("widget").unwrap().name = String::new();
        let v = check(&s);
        assert!(v
            .iter()
            .any(|x| matches!(x, Violation::EmptyModulePath { .. })));
    }

    #[test]
    fn detects_stale_schema_version() {
        let mut s = ok_spec();
        s.version = 0;
        let v = check(&s);
        assert!(v
            .iter()
            .any(|x| matches!(x, Violation::StaleSchemaVersion { .. })));
    }
}
