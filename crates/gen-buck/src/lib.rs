//! `gen-buck` — Buck2 BUCK renderer for the gen engine.
//!
//! Reuses the typed Starlark AST published by `gen-bazel` (BUCK
//! files are a Starlark dialect). Differences from Bazel:
//!   - rule names: `rust_library` is the same; `rust_binary` → same;
//!     `rust_test` → same. Buck adds `rust_unittest`.
//!   - external deps: `//third-party/rust:foo` instead of
//!     `@crate_index//:foo`.
//!   - prelude load path: `//prelude:rules.bzl` (or none — Buck loads
//!     prelude implicitly per project config).
//!
//! The render strategy is to compose typed StarlarkStmts (from
//! gen-bazel::ast) with Buck-flavored rule + dep strings, then pretty-
//! print via gen-bazel's renderer. Same AST surface, different name
//! resolution.

use gen_bazel::{KwArg, StarlarkStmt, StarlarkValue, render_file};
use gen_types::{Manifest, Package};

/// Render a typed Manifest to a Buck2 BUCK file body. One
/// `rust_library` call per workspace member.
#[must_use]
pub fn render_buck(manifest: &Manifest) -> String {
    let mut stmts: Vec<StarlarkStmt> = Vec::new();
    // Buck2 typically loads prelude implicitly via .buckconfig; we
    // emit no `load(...)` to mirror what real BUCK files look like.
    // Authors who need an explicit load can configure shikumi's
    // RenderConfig (M2 enrichment).
    for pkg in &manifest.packages {
        stmts.push(StarlarkStmt::Call {
            func: "rust_library".to_string(),
            args: rust_library_args(pkg),
        });
    }
    render_file(&stmts)
}

fn rust_library_args(pkg: &Package) -> Vec<KwArg> {
    vec![
        KwArg::str("name", &pkg.name),
        KwArg::positional_named(
            "srcs",
            StarlarkValue::Call {
                func: "glob".to_string(),
                args: vec![KwArg::positional(StarlarkValue::List(vec![
                    StarlarkValue::str("src/**/*.rs"),
                ]))],
            },
        ),
        KwArg::str("edition", "2021"),
        KwArg::positional_named(
            "deps",
            StarlarkValue::List(
                pkg.dependencies
                    .iter()
                    .map(|d| StarlarkValue::str(format!("//third-party/rust:{}", d.name)))
                    .collect(),
            ),
        ),
        KwArg::positional_named(
            "visibility",
            StarlarkValue::List(vec![StarlarkValue::str("PUBLIC")]),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use gen_types::*;

    fn demo() -> Manifest {
        let p = Package {
            name: "demo".into(),
            version: Version::new(0, 1, 0),
            source: PackageSource::Path { path: ".".into() },
            registry: Registry::CratesIo,
            dependencies: vec![Dependency {
                name: "serde".into(),
                constraint: VersionConstraint::from_spec(ConstraintSpec::Caret(Version::new(
                    1, 0, 0,
                ))),
                kind: DependencyKind::Direct,
                features_enabled: vec![],
                default_features: true,
                target_predicate: None,
                source_override: None,
            }],
            features: vec![],
            build_steps: vec![],
            license: None,
            description: None,
            authors: vec![],
            homepage: None,
            repository: None,
        };
        Manifest::new(
            "/x",
            Workspace::single_package("/x", "cargo"),
            vec![p],
            None,
        )
    }

    #[test]
    fn renders_buck_with_rust_library() {
        let s = render_buck(&demo());
        assert!(s.contains("rust_library("));
        assert!(s.contains("name = \"demo\""));
        // Buck uses //third-party/rust:name dep convention
        assert!(s.contains("//third-party/rust:serde"));
        // Buck uses PUBLIC visibility token, not Bazel's //visibility:public
        assert!(s.contains("\"PUBLIC\""));
    }

    #[test]
    fn renderer_is_deterministic() {
        let m = demo();
        assert_eq!(render_buck(&m), render_buck(&m));
    }
}
