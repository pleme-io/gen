//! `gen-bazel` — Bazel renderer for the gen engine.
//!
//! Same typed Manifest as gen-nix / gen-nix-bulk; emits a typed
//! Starlark AST → canonical pretty-printer → BUILD.bazel +
//! MODULE.bazel files. The destination per the GEN.md plan:
//! one source-of-truth Manifest, N typed renderers (Nix / Bazel /
//! Buck / Bazel-rust-rules), operators pick the backend via shikumi.
//!
//! The Starlark AST is intentionally minimal (atoms / lists / dicts /
//! function-calls / assignments) — matches what BUILD files actually
//! use; full Starlark eval is not in scope.

pub mod ast;
pub mod render;

pub use ast::{StarlarkValue, StarlarkStmt, KwArg};
pub use render::render_file;

use gen_types::{Manifest, Package};

/// Render a typed Manifest to a BUILD.bazel file body. One
/// `rust_library` or `rust_binary` call per workspace member, plus
/// `rust_test` for the integration tests when present.
#[must_use]
pub fn render_build_bazel(manifest: &Manifest) -> String {
    let mut stmts: Vec<StarlarkStmt> = Vec::new();
    stmts.push(StarlarkStmt::Load {
        module: "@rules_rust//rust:defs.bzl".to_string(),
        symbols: vec!["rust_library".into(), "rust_binary".into()],
    });
    for pkg in &manifest.packages {
        stmts.push(StarlarkStmt::Call {
            func: "rust_library".to_string(),
            args: rust_library_args(pkg),
        });
    }
    render::render_file(&stmts)
}

/// Render a MODULE.bazel that declares the cargo dependencies via
/// the `cargo` extension. Each external crate becomes one
/// `crate.from_cargo` entry; operators use `bazel mod tidy` after to
/// pin transitives.
#[must_use]
pub fn render_module_bazel(manifest: &Manifest) -> String {
    let mut stmts: Vec<StarlarkStmt> = Vec::new();
    let module_name = manifest
        .packages
        .first()
        .map(|p| p.name.clone())
        .unwrap_or_else(|| "workspace".to_string());
    let module_version = manifest
        .packages
        .first()
        .map(|p| p.version.to_string())
        .unwrap_or_else(|| "0.1.0".to_string());
    stmts.push(StarlarkStmt::Call {
        func: "module".to_string(),
        args: vec![
            KwArg::str("name", &module_name),
            KwArg::str("version", &module_version),
        ],
    });
    stmts.push(StarlarkStmt::Assign {
        name: "rust".to_string(),
        value: StarlarkValue::Call {
            func: "use_extension".to_string(),
            args: vec![
                KwArg::positional(StarlarkValue::str("@rules_rust//rust:extensions.bzl")),
                KwArg::positional(StarlarkValue::str("rust")),
            ],
        },
    });
    stmts.push(StarlarkStmt::Call {
        func: "use_repo".to_string(),
        args: vec![
            KwArg::positional(StarlarkValue::Ident("rust".to_string())),
            KwArg::positional(StarlarkValue::str("rust_toolchains")),
        ],
    });
    render::render_file(&stmts)
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
                    .map(|d| StarlarkValue::str(format!("@crate_index//:{}", d.name)))
                    .collect(),
            ),
        ),
        KwArg::positional_named(
            "visibility",
            StarlarkValue::List(vec![StarlarkValue::str("//visibility:public")]),
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
                constraint: VersionConstraint::from_spec(ConstraintSpec::Caret(Version::new(1, 0, 0))),
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
        Manifest::new("/x", Workspace::single_package("/x", "cargo"), vec![p], None)
    }

    #[test]
    fn renders_build_bazel_with_rust_library() {
        let s = render_build_bazel(&demo());
        assert!(s.contains("load(\"@rules_rust//rust:defs.bzl\""));
        assert!(s.contains("rust_library("));
        assert!(s.contains("name = \"demo\""));
        assert!(s.contains("@crate_index//:serde"));
    }

    #[test]
    fn renders_module_bazel_with_module_call_and_use_extension() {
        let s = render_module_bazel(&demo());
        assert!(s.contains("module(name = \"demo\""));
        assert!(s.contains("rust = use_extension("));
        assert!(s.contains("use_repo(rust"));
    }

    #[test]
    fn renderer_is_deterministic() {
        let m = demo();
        let a = render_build_bazel(&m);
        let b = render_build_bazel(&m);
        assert_eq!(a, b);
    }
}
