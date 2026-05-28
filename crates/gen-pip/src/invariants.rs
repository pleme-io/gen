//! Invariants over the pip `BuildSpec`. Implements
//! `gen_types::Invariants` so cse-lint + gen confirm can call into
//! the adapter uniformly.

use serde::{Deserialize, Serialize};

use crate::build_spec::BuildSpec;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "rule", rename_all = "kebab-case")]
pub enum Violation {
    StaleSchemaVersion { found: u32, expected: u32 },
    // TODO: ecosystem-specific violations.
}

pub fn check(spec: &BuildSpec) -> Vec<Violation> {
    let mut out = Vec::new();
    if spec.version < crate::build_spec::SCHEMA_VERSION {
        out.push(Violation::StaleSchemaVersion {
            found: spec.version,
            expected: crate::build_spec::SCHEMA_VERSION,
        });
    }
    out
}

pub struct PipInvariants;

impl gen_types::Invariants for PipInvariants {
    type Spec = BuildSpec;
    type Violation = Violation;
    fn check(spec: &Self::Spec) -> Vec<Self::Violation> {
        check(spec)
    }
}
