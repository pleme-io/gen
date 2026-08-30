//! Typed [`Derivation`] — the renderer-side output.
//!
//! Every renderer (`gen-nix-incremental`, `gen-nix-bulk`, `gen-bazel`,
//! `gen-buck`) emits these. Cache backends key on
//! `Derivation.cache_key`. Build systems consume `Derivation.build`.
//!
//! Per `theory/NIX-AST.md`: the Nix renderer never `format!()`s nix
//! syntax — it emits a typed AST that pretty-prints deterministically.
//! Other build systems get their own typed AST module under their
//! renderer crate.

use crate::lockfile::ContentHash;
use crate::{PackageId, Version};
use serde::{Deserialize, Serialize};

/// One derivation in the renderer's output graph. Build system
/// agnostic.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Derivation {
    pub name: String,
    pub version: Version,
    /// Source package this derivation was emitted for (one-to-one).
    pub package_id: PackageId,
    /// Inputs the build needs (resolved at the renderer's layer).
    pub inputs: Vec<DerivationRef>,
    /// Ordered build steps the build system executes.
    pub build: BuildScript,
    /// Canonical hash for cache-backend lookup. Computed by the
    /// renderer over the derivation's content-addressed inputs +
    /// build script — so a cache backend can answer "has this
    /// already been built?" without consulting the source.
    pub cache_key: ContentHash,
}

/// Reference to another derivation by its cache key. Renderers
/// resolve these into per-build-system specific shapes (Nix store
/// paths, Bazel targets, …).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DerivationRef {
    pub name: String,
    pub cache_key: ContentHash,
}

/// Typed build script — a sequence of [`BuildStep`]s + the
/// environment they share.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct BuildScript {
    pub steps: Vec<BuildStep>,
    /// Environment variables exported across every step.
    #[serde(default)]
    pub env: indexmap::IndexMap<String, String>,
}

/// One phase of the build script.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildStep {
    pub kind: BuildStepKind,
    pub command: BuildCommand,
}

/// Typed enum of standard build phases. Adapters / renderers don't
/// invent new variants; they pick from this list. Pre/PostX hooks
/// are the only place adapter-specific logic surfaces.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum BuildStepKind {
    Fetch,
    PreUnpack,
    Unpack,
    PostUnpack,
    Patch,
    PreConfigure,
    Configure,
    PostConfigure,
    PreBuild,
    Build,
    PostBuild,
    PreCheck,
    Check,
    PostCheck,
    PreInstall,
    Install,
    PostInstall,
    /// Custom adapter-defined phase. Engine treats it as opaque +
    /// runs it at the position the adapter requested. Name is the
    /// adapter's chosen label (e.g. `"strip"`, `"sign"`,
    /// `"cargo:rustc-link-lib"`).
    Custom {
        name: String,
    },
}

/// Declarative build command — NOT raw shell. Renderers translate
/// to the build system's preferred shape (Nix derivation phases,
/// Bazel `genrule`, …).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum BuildCommand {
    /// A typed cargo invocation: `cargo build --release` etc.
    Cargo {
        subcommand: String,
        args: Vec<String>,
    },
    /// A typed npm/pnpm/yarn invocation.
    Npm {
        manager: String,
        subcommand: String,
        args: Vec<String>,
    },
    /// A typed pip invocation.
    Pip {
        subcommand: String,
        args: Vec<String>,
    },
    /// A typed gem invocation.
    Gem {
        subcommand: String,
        args: Vec<String>,
    },
    /// A typed go invocation.
    Go {
        subcommand: String,
        args: Vec<String>,
    },
    /// A typed `make`-shaped target.
    Make {
        target: String,
        env: indexmap::IndexMap<String, String>,
    },
    /// A typed file copy step (Fetch / Install phase).
    Copy { from: String, to: String },
    /// A typed mkdir step.
    Mkdir { path: String },
    /// A typed symlink step.
    Symlink { target: String, link_name: String },
    /// Adapter-supplied native command — used as the last resort
    /// when the typed variants don't cover it. Renderer is
    /// expected to translate, not pass through raw.
    Native { program: String, args: Vec<String> },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Registry;

    #[test]
    fn derivation_round_trips_through_serde() {
        let d = Derivation {
            name: "serde-1.0.228".into(),
            version: Version::new(1, 0, 228),
            package_id: PackageId {
                name: "serde".into(),
                version: Version::new(1, 0, 228),
                registry: Registry::CratesIo,
            },
            inputs: vec![],
            build: BuildScript {
                steps: vec![BuildStep {
                    kind: BuildStepKind::Build,
                    command: BuildCommand::Cargo {
                        subcommand: "build".into(),
                        args: vec!["--release".into()],
                    },
                }],
                env: indexmap::IndexMap::new(),
            },
            cache_key: ContentHash::of(b"snapshot"),
        };
        let j = serde_json::to_string(&d).unwrap();
        let parsed: Derivation = serde_json::from_str(&j).unwrap();
        assert_eq!(d, parsed);
    }

    #[test]
    fn build_command_variants_round_trip() {
        let commands = vec![
            BuildCommand::Cargo {
                subcommand: "build".into(),
                args: vec![],
            },
            BuildCommand::Npm {
                manager: "pnpm".into(),
                subcommand: "install".into(),
                args: vec![],
            },
            BuildCommand::Copy {
                from: "/src".into(),
                to: "/out".into(),
            },
        ];
        for c in commands {
            let j = serde_json::to_string(&c).unwrap();
            let parsed: BuildCommand = serde_json::from_str(&j).unwrap();
            assert_eq!(c, parsed);
        }
    }
}
