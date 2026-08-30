//! Invariants over the npm `BuildSpec`. Implements
//! `gen_types::Invariants` so cse-lint + gen confirm can call into
//! the adapter uniformly.

use serde::{Deserialize, Serialize};

use crate::build_spec::BuildSpec;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "rule", rename_all = "kebab-case")]
pub enum Violation {
    StaleSchemaVersion {
        found: u32,
        expected: u32,
    },
    MissingPname {
        package_key: String,
    },
    MissingVersion {
        package_key: String,
    },
    /// A package has `npmDepsHash` empty + `dontNpmBuild` unset.
    /// Combined, that means the build phase will run but the
    /// prefetch is unresolved — a contradiction.
    UnresolvedDeps {
        package_key: String,
        name: String,
    },
}

pub fn check(spec: &BuildSpec) -> Vec<Violation> {
    let mut out = Vec::new();
    if spec.version < crate::build_spec::SCHEMA_VERSION {
        out.push(Violation::StaleSchemaVersion {
            found: spec.version,
            expected: crate::build_spec::SCHEMA_VERSION,
        });
    }
    for (key, p) in &spec.packages {
        if p.args.pname.is_none() {
            out.push(Violation::MissingPname {
                package_key: key.clone(),
            });
        }
        if p.args.version.is_none() {
            out.push(Violation::MissingVersion {
                package_key: key.clone(),
            });
        }
        let want_build = !matches!(p.args.dont_npm_build, Some(true));
        if want_build
            && p.args
                .npm_deps_hash
                .as_deref()
                .map(|s| s.is_empty())
                .unwrap_or(true)
        {
            // Allow missing hash entirely; only flag when present-but-empty.
            if p.args.npm_deps_hash.is_some() {
                out.push(Violation::UnresolvedDeps {
                    package_key: key.clone(),
                    name: p.name.clone(),
                });
            }
        }
    }
    out
}

pub struct NpmInvariants;

impl gen_types::Invariants for NpmInvariants {
    type Spec = BuildSpec;
    type Violation = Violation;
    fn check(spec: &Self::Spec) -> Vec<Self::Violation> {
        check(spec)
    }
}
