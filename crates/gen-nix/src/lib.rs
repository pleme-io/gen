//! `gen-nix` — typed Nix AST + canonical pretty-printer + per-crate
//! derivation renderer (crate2nix shape).
//!
//! Implements `theory/NIX-AST.md`: every Nix construct emitted by the
//! gen engine is a typed [`NixValue`] rendered through one
//! [`render::render`] function. `format!()` of nix syntax is forbidden
//! in downstream consumers.

pub mod ast;
pub mod cargo_derivation;
pub mod render;

pub use ast::{
    dotted_entry, entry, AttrKey, AttrPath, AttrSetEntry, LambdaParams, LetBinding, NixBinOp,
    NixUnaryOp, NixValue, ParamField, StrPart,
};
pub use cargo_derivation::render_workspace;
pub use render::render;

#[cfg(test)]
mod tests {
    use super::*;

    fn r(v: &NixValue) -> String {
        v.render_to_string()
    }

    // ── Atoms ────────────────────────────────────────────────────

    #[test]
    fn renders_atoms() {
        assert_eq!(r(&NixValue::Null), "null");
        assert_eq!(r(&NixValue::Bool(true)), "true");
        assert_eq!(r(&NixValue::Bool(false)), "false");
        assert_eq!(r(&NixValue::Int(42)), "42");
        assert_eq!(r(&NixValue::Str("hello".into())), "\"hello\"");
        assert_eq!(r(&NixValue::Path("./foo".into())), "./foo");
        assert_eq!(r(&NixValue::Ident("pkgs".into())), "pkgs");
    }

    #[test]
    fn renders_string_with_escapes() {
        let v = NixValue::Str(r#"a"b\c$d"#.into());
        assert_eq!(r(&v), r#""a\"b\\c\$d""#);
    }

    #[test]
    fn renders_interpolated_str() {
        let v = NixValue::InterpolatedStr(vec![
            StrPart::Literal("pre ".into()),
            StrPart::Interp(NixValue::Ident("pkgs".into())),
            StrPart::Literal("/bin".into()),
        ]);
        assert_eq!(r(&v), "\"pre ${pkgs}/bin\"");
    }

    #[test]
    fn renders_indented_str() {
        let v = NixValue::IndentedStr(vec!["line 1".into(), "line 2".into()]);
        let s = r(&v);
        assert!(s.starts_with("''"));
        assert!(s.contains("line 1"));
        assert!(s.contains("line 2"));
        assert!(s.ends_with("''"));
    }

    #[test]
    fn renders_attr_path() {
        let v = NixValue::AttrPath(vec!["pkgs".into(), "lib".into(), "x".into()]);
        assert_eq!(r(&v), "pkgs.lib.x");
    }

    // ── Collections ──────────────────────────────────────────────

    #[test]
    fn renders_empty_list_and_attrset() {
        assert_eq!(r(&NixValue::List(vec![])), "[ ]");
        let empty = NixValue::AttrSet {
            recursive: false,
            entries: vec![],
        };
        assert_eq!(r(&empty), "{ }");
    }

    #[test]
    fn renders_short_atomic_list_inline() {
        let v = NixValue::List(vec![
            NixValue::Int(1),
            NixValue::Int(2),
            NixValue::Int(3),
        ]);
        assert_eq!(r(&v), "[ 1 2 3 ]");
    }

    #[test]
    fn renders_long_list_block_form() {
        let items: Vec<NixValue> = (0..10).map(NixValue::Int).collect();
        let v = NixValue::List(items);
        let s = r(&v);
        assert!(s.starts_with('['));
        assert!(s.ends_with(']'));
        assert!(s.contains("\n"));
    }

    #[test]
    fn renders_simple_attrset() {
        let v = NixValue::attrset([
            ("a".to_string(), NixValue::Int(1)),
            ("b".to_string(), NixValue::Str("two".into())),
        ]);
        let s = r(&v);
        assert!(s.contains("a = 1;"));
        assert!(s.contains("b = \"two\";"));
    }

    #[test]
    fn renders_rec_attrset() {
        let v = NixValue::rec_attrset([("x".to_string(), NixValue::Int(1))]);
        assert!(r(&v).starts_with("rec {"));
    }

    #[test]
    fn renders_dotted_entry() {
        let v = NixValue::AttrSet {
            recursive: false,
            entries: vec![dotted_entry("a.b.c", NixValue::Int(7))],
        };
        assert!(r(&v).contains("a.b.c = 7;"));
    }

    #[test]
    fn renders_inherit_entry() {
        let v = NixValue::AttrSet {
            recursive: false,
            entries: vec![AttrSetEntry::Inherit {
                from: Some(NixValue::Ident("pkgs".into())),
                names: vec!["foo".into(), "bar".into()],
            }],
        };
        let s = r(&v);
        assert!(s.contains("inherit (pkgs) foo bar;"));
    }

    // ── Functions ────────────────────────────────────────────────

    #[test]
    fn renders_single_param_lambda() {
        let v = NixValue::lambda_single("x", NixValue::Ident("x".into()));
        assert_eq!(r(&v), "x: x");
    }

    #[test]
    fn renders_destructured_lambda_with_ellipsis() {
        let v = NixValue::Lambda {
            params: LambdaParams::Destructured {
                fields: vec![
                    ParamField {
                        name: "pkgs".into(),
                        default: None,
                    },
                    ParamField {
                        name: "extra".into(),
                        default: Some(NixValue::Int(0)),
                    },
                ],
                ellipsis: true,
                binding: None,
            },
            body: Box::new(NixValue::Ident("pkgs".into())),
        };
        assert_eq!(r(&v), "{ pkgs, extra ? 0, ... }: pkgs");
    }

    #[test]
    fn renders_apply() {
        let v = NixValue::apply(
            NixValue::Ident("fn".into()),
            [NixValue::Int(1), NixValue::Int(2)],
        );
        assert_eq!(r(&v), "fn 1 2");
    }

    #[test]
    fn application_parenthesizes_nested_apply_arg() {
        let v = NixValue::apply(
            NixValue::Ident("g".into()),
            [NixValue::apply(
                NixValue::Ident("f".into()),
                [NixValue::Int(1)],
            )],
        );
        assert_eq!(r(&v), "g (f 1)");
    }

    // ── Control + operators ──────────────────────────────────────

    #[test]
    fn renders_let_in() {
        let v = NixValue::Let {
            bindings: vec![LetBinding::Bind {
                name: "x".into(),
                value: NixValue::Int(1),
            }],
            body: Box::new(NixValue::Ident("x".into())),
        };
        let s = r(&v);
        assert!(s.contains("let"));
        assert!(s.contains("x = 1;"));
        assert!(s.contains("in"));
    }

    #[test]
    fn renders_with() {
        let v = NixValue::With {
            scope: Box::new(NixValue::Ident("pkgs".into())),
            body: Box::new(NixValue::Ident("hello".into())),
        };
        assert_eq!(r(&v), "with pkgs; hello");
    }

    #[test]
    fn renders_if() {
        let v = NixValue::If {
            cond: Box::new(NixValue::Bool(true)),
            then_branch: Box::new(NixValue::Int(1)),
            else_branch: Box::new(NixValue::Int(2)),
        };
        assert_eq!(r(&v), "if true then 1 else 2");
    }

    #[test]
    fn binop_precedence_avoids_redundant_parens() {
        // 1 + 2 * 3 → multiplication binds tighter, no parens needed
        let v = NixValue::BinOp {
            op: NixBinOp::Add,
            left: Box::new(NixValue::Int(1)),
            right: Box::new(NixValue::BinOp {
                op: NixBinOp::Mul,
                left: Box::new(NixValue::Int(2)),
                right: Box::new(NixValue::Int(3)),
            }),
        };
        assert_eq!(r(&v), "1 + 2 * 3");
    }

    #[test]
    fn binop_precedence_adds_parens_when_needed() {
        // (1 + 2) * 3 — addition is looser, must be parenthesized
        let v = NixValue::BinOp {
            op: NixBinOp::Mul,
            left: Box::new(NixValue::BinOp {
                op: NixBinOp::Add,
                left: Box::new(NixValue::Int(1)),
                right: Box::new(NixValue::Int(2)),
            }),
            right: Box::new(NixValue::Int(3)),
        };
        assert_eq!(r(&v), "(1 + 2) * 3");
    }

    #[test]
    fn renders_attr_or() {
        let v = NixValue::AttrOr {
            attrset: Box::new(NixValue::Ident("pkgs".into())),
            attr: vec!["foo".into(), "bar".into()],
            default: Box::new(NixValue::Null),
        };
        assert_eq!(r(&v), "pkgs.foo.bar or null");
    }

    #[test]
    fn renders_has_attr() {
        let v = NixValue::HasAttr {
            attrset: Box::new(NixValue::Ident("x".into())),
            attr: vec!["a".into()],
        };
        assert_eq!(r(&v), "x ? a");
    }

    // ── End-to-end: cargo derivation render ──────────────────────

    #[test]
    fn renders_a_workspace_with_one_member_and_one_resolved() {
        use gen_types::{
            ContentHash, Lockfile, Manifest, Package, PackageId, PackageSource, Registry,
            ResolvedPackage, Version, Workspace,
        };
        use indexmap::IndexMap;

        let pkg = Package {
            name: "demo".into(),
            version: Version::new(0, 1, 0),
            source: PackageSource::Path {
                path: "./crates/demo".into(),
            },
            registry: Registry::CratesIo,
            dependencies: vec![],
            features: vec![],
            build_steps: vec![],
            license: Some("MIT".into()),
            description: None,
            authors: vec![],
            homepage: None,
            repository: None,
        };
        let mut resolved = IndexMap::new();
        resolved.insert(
            "serde/1.0.228".to_string(),
            ResolvedPackage {
                id: PackageId {
                    name: "serde".into(),
                    version: Version::new(1, 0, 228),
                    registry: Registry::CratesIo,
                },
                source: PackageSource::Registry {
                    registry: Registry::CratesIo,
                    registry_name: "serde".into(),
                    integrity_hash: Some("sha256:abc".into()),
                },
                integrity: Some("sha256:abc".into()),
                resolved_dependencies: vec![],
            },
        );
        let lock = Lockfile {
            resolved,
            content_addressed_hash: ContentHash::genesis(),
        };
        let m = Manifest::new(
            "/x",
            Workspace::single_package("/x", "cargo"),
            vec![pkg],
            Some(lock),
        );

        let nix = render_workspace(&m);
        let s = nix.render_to_string();
        // Lambda over { pkgs, buildRustCrate, ... }
        assert!(s.starts_with("{ pkgs, buildRustCrate, ... }: "));
        // Has the crates attrset
        assert!(s.contains("crates"));
        // Resolved entry uses serde-1.0.228 key
        assert!(s.contains("serde-1.0.228"));
        // Workspace member appears under -workspace- suffix
        assert!(s.contains("demo-workspace-0.1.0"));
        // workspace_members list emitted
        assert!(s.contains("workspace_members"));
        // buildRustCrate calls visible
        assert!(s.contains("buildRustCrate {"));
    }

    #[test]
    fn render_is_deterministic() {
        let v = NixValue::attrset([
            ("z".to_string(), NixValue::Int(1)),
            ("a".to_string(), NixValue::Int(2)),
        ]);
        assert_eq!(r(&v), r(&v.clone()));
    }
}
