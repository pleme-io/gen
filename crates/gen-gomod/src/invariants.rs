//! Algorithmic invariants over the gomod v2 `BuildSpec`.
//!
//! These are PROVEN PROPERTIES of every spec the encoder emits — the
//! encoder-side half of the GEN TYPED-SPEC CONTRACT (each conditional
//! emission rule is a typed invariant, asserted here and defended on the
//! substrate interpreter side). `check(&spec)` is pure, deterministic,
//! side-effect-free; violations are bugs in the encoder, never in
//! downstream consumers.
//!
//! Invariant map (see the M1 build doc §6):
//! - **Go-I1** every import edge resolves to a node in `packages`.
//! - **Go-I3** a `Vendored` source's `relative_path` is a real subdir
//!   (no `..`, not absolute).
//! - **Go-I8** every non-std node carries a `source_hash`.
//! - **Go-I9** a node with embed patterns carries embed files.
//! - **Go-I10** std nodes ⟺ `Std` source; non-std nodes never `Std`.
//! - **Go-I11** every `workspace_member` is a `Main` node in `packages`.
//! - **Go-I12** (cgo/asm exclusion) is enforced at ENCODE time
//!   (`interp::apply` fails closed) and is *structurally* unrepresentable
//!   in the M1 [`PackageKind`] enum — there is no `Cgo`/`Tool` arm to
//!   check post-facto, so no runtime invariant is needed.

use serde::{Deserialize, Serialize};

use crate::build_spec::{BuildSpec, PackageKind, PackageSource};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "rule", rename_all = "kebab-case")]
pub enum Violation {
    /// Spec emitted below the current `SCHEMA_VERSION`.
    StaleSchemaVersion { found: u32, expected: u32 },
    /// `root_package` names a key not present in `packages`.
    RootNotInPackages { key: String },
    /// A `workspace_member` names a key not present in `packages`.
    WorkspaceMemberNotInPackages { key: String },
    /// A `workspace_member` is not a `Main` node — only linkable
    /// binaries are members (Go-I11).
    WorkspaceMemberNotMain { key: String },
    /// Go-I1: an import edge references a node not present in `packages`.
    UnresolvedImport { from: String, missing_key: String },
    /// Go-I10: a `Std`-kind node carries a `Vendored` source.
    StdNodeHasVendoredSource { key: String },
    /// Go-I10: a non-`Std` node carries a `Std` source.
    NonStdNodeHasStdSource { key: String },
    /// Go-I8: a non-std node has an empty `source_hash` (no content
    /// address ⇒ no incremental cache key).
    NodeMissingSourceHash { key: String },
    /// Go-I3: a `Vendored` `relative_path` escapes the workspace src
    /// (absolute, or contains a `..` component).
    RelativePathEscapes { key: String, relative_path: String },
    /// Go-I9: a node declares embed patterns but no resolved embed
    /// files — the interpreter would emit no `-embedcfg` and the
    /// `//go:embed` compile would fail.
    EmbedPatternsWithoutFiles { key: String },
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

    if !spec.root_package.is_empty() && !spec.packages.contains_key(&spec.root_package) {
        out.push(Violation::RootNotInPackages {
            key: spec.root_package.clone(),
        });
    }

    for m in &spec.workspace_members {
        match spec.packages.get(m) {
            None => out.push(Violation::WorkspaceMemberNotInPackages { key: m.clone() }),
            Some(p) if !p.kind.is_main() => {
                out.push(Violation::WorkspaceMemberNotMain { key: m.clone() });
            }
            _ => {}
        }
    }

    for (key, p) in &spec.packages {
        // Go-I10 — kind ⟺ source.
        match (p.kind, &p.source) {
            (PackageKind::Std, PackageSource::Vendored { .. }) => {
                out.push(Violation::StdNodeHasVendoredSource { key: key.clone() });
            }
            (k, PackageSource::Std) if !k.is_std() => {
                out.push(Violation::NonStdNodeHasStdSource { key: key.clone() });
            }
            _ => {}
        }

        // Go-I8 — non-std node carries a content address.
        if !p.kind.is_std() && p.source_hash.is_empty() {
            out.push(Violation::NodeMissingSourceHash { key: key.clone() });
        }

        // Go-I3 — relative_path stays inside the workspace src.
        if let PackageSource::Vendored { relative_path } = &p.source {
            if relative_path.starts_with('/') || relative_path.split('/').any(|c| c == "..") {
                out.push(Violation::RelativePathEscapes {
                    key: key.clone(),
                    relative_path: relative_path.clone(),
                });
            }
        }

        // Go-I1 — every edge resolves.
        for edge in &p.imports {
            if !spec.packages.contains_key(edge) {
                out.push(Violation::UnresolvedImport {
                    from: key.clone(),
                    missing_key: edge.clone(),
                });
            }
        }

        // Go-I9 — embed patterns imply embed files.
        if !p.embed.patterns.is_empty() && p.embed.files.is_empty() {
            out.push(Violation::EmbedPatternsWithoutFiles { key: key.clone() });
        }
    }

    out
}

/// Stable rule-name + optional locus for a violation — used by the
/// adapter's `confirm` typed report.
#[must_use]
pub fn violation_locus(v: &Violation) -> (&'static str, Option<String>) {
    use Violation::*;
    match v {
        StaleSchemaVersion { .. } => ("stale-schema-version", None),
        RootNotInPackages { key } => ("root-not-in-packages", Some(key.clone())),
        WorkspaceMemberNotInPackages { key } => {
            ("workspace-member-not-in-packages", Some(key.clone()))
        }
        WorkspaceMemberNotMain { key } => ("workspace-member-not-main", Some(key.clone())),
        UnresolvedImport { from, .. } => ("unresolved-import", Some(from.clone())),
        StdNodeHasVendoredSource { key } => ("std-node-has-vendored-source", Some(key.clone())),
        NonStdNodeHasStdSource { key } => ("non-std-node-has-std-source", Some(key.clone())),
        NodeMissingSourceHash { key } => ("node-missing-source-hash", Some(key.clone())),
        RelativePathEscapes { key, .. } => ("relative-path-escapes", Some(key.clone())),
        EmbedPatternsWithoutFiles { key } => ("embed-patterns-without-files", Some(key.clone())),
    }
}

pub struct GomodInvariants;

impl gen_types::Invariants for GomodInvariants {
    type Spec = BuildSpec;
    type Violation = Violation;
    fn check(spec: &Self::Spec) -> Vec<Self::Violation> {
        check(spec)
    }
}
