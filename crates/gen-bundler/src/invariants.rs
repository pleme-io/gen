use serde::{Deserialize, Serialize};

use crate::build_spec::BuildSpec;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "rule", rename_all = "kebab-case")]
pub enum Violation {
    StaleSchemaVersion { found: u32, expected: u32 },
    MissingPname { package_key: String },
    MissingVersion { package_key: String },
    /// `bundlerApp` requires `exes` non-empty; a spec declaring app
    /// shape without exes is mis-shaped.
    AppMissingExes { package_key: String },
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
    }
    out
}

pub struct BundlerInvariants;

impl gen_types::Invariants for BundlerInvariants {
    type Spec = BuildSpec;
    type Violation = Violation;
    fn check(spec: &Self::Spec) -> Vec<Self::Violation> {
        check(spec)
    }
}
