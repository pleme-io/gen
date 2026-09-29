//! Canonical pretty-printer for [`NixValue`]. Deterministic +
//! idempotent + nixpkgs-style indent. The only function downstream
//! consumers call to turn a typed AST into Nix source.

use crate::ast::{
    AttrKey, AttrPath, AttrSetEntry, CollectionLayout, LambdaLayout, LambdaParams, LetBinding,
    LetLayout, NixBinOp, NixValue, ParamField, StrPart,
};

const INDENT_STEP: usize = 2;

/// Render `value` to canonical Nix source. Entry point.
pub fn render(value: &NixValue) -> String {
    let mut out = String::new();
    write_value(&mut out, value, 0, u8::MAX);
    out
}

/// Render `value` as a whole `.nix` file: [`render`] plus the trailing
/// newline a file ends with.
#[must_use]
pub fn render_file(value: &NixValue) -> String {
    let mut out = render(value);
    out.push('\n');
    out
}

fn indent_str(level: usize) -> String {
    " ".repeat(level * INDENT_STEP)
}

fn write_value(out: &mut String, value: &NixValue, level: usize, parent_prec: u8) {
    match value {
        NixValue::Null => out.push_str("null"),
        NixValue::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        NixValue::Int(i) => out.push_str(&i.to_string()),
        NixValue::Float(f) => out.push_str(&f.to_string()),
        NixValue::Str(s) => write_quoted_str(out, s),
        NixValue::IndentedStr(lines) => write_indented_str(out, lines, level),
        NixValue::Path(p) => out.push_str(p),
        NixValue::InterpolatedStr(parts) => write_interpolated_str(out, parts, level),
        NixValue::Ident(s) => out.push_str(s),
        NixValue::AttrPath(parts) => out.push_str(&parts.join(".")),
        NixValue::List(items) => write_list(out, items, level),
        NixValue::AttrSet { recursive, entries } => write_attrset(out, *recursive, entries, level),
        NixValue::Lambda { params, body } => write_lambda(out, params, body, level),
        NixValue::Apply { func, args } => write_apply(out, func, args, level, parent_prec),
        NixValue::Let { bindings, body } => write_let(out, bindings, body, level),
        NixValue::With { scope, body } => {
            out.push_str("with ");
            write_value(out, scope, level, u8::MAX);
            out.push_str("; ");
            write_value(out, body, level, u8::MAX);
        }
        NixValue::If {
            cond,
            then_branch,
            else_branch,
        } => {
            out.push_str("if ");
            write_value(out, cond, level, u8::MAX);
            out.push_str(" then ");
            write_value(out, then_branch, level, u8::MAX);
            out.push_str(" else ");
            write_value(out, else_branch, level, u8::MAX);
        }
        NixValue::BinOp { op, left, right } => {
            write_binop(out, *op, left, right, level, parent_prec)
        }
        NixValue::UnaryOp { op, operand } => {
            out.push_str(op.as_str());
            write_value(out, operand, level, 0);
        }
        NixValue::AttrOr {
            attrset,
            attr,
            default,
        } => {
            write_value(out, attrset, level, u8::MAX);
            out.push('.');
            out.push_str(&attr.join("."));
            out.push_str(" or ");
            write_value(out, default, level, u8::MAX);
        }
        NixValue::HasAttr { attrset, attr } => {
            write_value(out, attrset, level, u8::MAX);
            out.push_str(" ? ");
            out.push_str(&attr.join("."));
        }
        NixValue::IndentedInterpolatedStr(lines) => {
            write_indented_interpolated_str(out, lines, level);
        }
        NixValue::Select {
            base,
            path,
            default,
        } => write_select(out, base, path, default.as_deref(), level),
        NixValue::Commented { comments, value } => {
            for c in comments {
                write_comment(out, c);
                out.push('\n');
                out.push_str(&indent_str(level));
            }
            write_value(out, value, level, parent_prec);
        }
        NixValue::LaidOutAttrSet { .. }
        | NixValue::LaidOutList { .. }
        | NixValue::LaidOutLet { .. }
        | NixValue::LaidOutLambda { .. } => write_laid_out(out, value, level),
        NixValue::Raw(s) => out.push_str(s),
    }
}

/// The layout siblings of attrset, list, let and lambda.
fn write_laid_out(out: &mut String, value: &NixValue, level: usize) {
    match value {
        NixValue::LaidOutAttrSet {
            recursive,
            entries,
            layout,
        } => {
            if *recursive {
                out.push_str("rec ");
            }
            match layout {
                CollectionLayout::Inline => write_inline_attrset(out, entries, level),
                CollectionLayout::Block => write_block_attrset(out, entries, level),
            }
        }
        NixValue::LaidOutList { items, layout } => match layout {
            CollectionLayout::Inline => write_inline_list(out, items, level),
            CollectionLayout::Block => write_block_list(out, items, level),
        },
        NixValue::LaidOutLet {
            bindings,
            body,
            layout,
        } => match layout {
            LetLayout::BodyOnNextLine => write_let(out, bindings, body, level),
            LetLayout::BodyAfterIn => {
                write_let_head(out, bindings, level);
                out.push_str("in ");
                write_value(out, body, level, u8::MAX);
            }
        },
        NixValue::LaidOutLambda {
            params,
            body,
            layout,
        } => match layout {
            LambdaLayout::BodyOnSameLine => write_lambda(out, params, body, level),
            LambdaLayout::BodyOnNextLine => {
                write_lambda_params(out, params, level);
                out.push_str(":\n");
                out.push_str(&indent_str(level));
                write_value(out, body, level, u8::MAX);
            }
        },
        _ => unreachable!("write_laid_out called with a canonical variant"),
    }
}

/// `# text`, or a bare `#` for an empty line.
fn write_comment(out: &mut String, text: &str) {
    out.push('#');
    if !text.is_empty() {
        out.push(' ');
        out.push_str(text);
    }
}

/// `/* text */`: the comment form that is safe mid-line.
fn write_inline_comment(out: &mut String, text: &str) {
    out.push_str("/* ");
    out.push_str(text);
    out.push_str(" */");
}

fn write_indented_interpolated_str(out: &mut String, lines: &[Vec<StrPart>], level: usize) {
    out.push_str("''");
    let inner_indent = indent_str(level + 1);
    for line in lines {
        out.push('\n');
        if line.is_empty() {
            continue;
        }
        out.push_str(&inner_indent);
        // Adjacent literals are escaped as one run, so a `'` ending one
        // part and a `'` starting the next still become `'''`.
        let mut literal = String::new();
        for part in line {
            match part {
                StrPart::Literal(s) => literal.push_str(s),
                StrPart::Interp(v) => {
                    write_indented_literal(out, &literal);
                    literal.clear();
                    out.push_str("${");
                    write_value(out, v, level + 1, u8::MAX);
                    out.push('}');
                }
            }
        }
        write_indented_literal(out, &literal);
    }
    out.push('\n');
    out.push_str(&indent_str(level));
    out.push_str("''");
}

/// Escape literal text for an indented string: `''` becomes `'''` and
/// `${` becomes `''${`; nothing else is special there.
fn write_indented_literal(out: &mut String, s: &str) {
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, chars.peek()) {
            ('\'', Some('\'')) => {
                chars.next();
                out.push_str("'''");
            }
            ('$', Some('{')) => {
                chars.next();
                out.push_str("''${");
            }
            _ => out.push(c),
        }
    }
}

fn write_select(
    out: &mut String,
    base: &NixValue,
    path: &AttrPath,
    default: Option<&NixValue>,
    level: usize,
) {
    let base_is_primary = matches!(
        base,
        NixValue::Ident(_)
            | NixValue::AttrPath(_)
            | NixValue::Select { default: None, .. }
            | NixValue::Str(_)
            | NixValue::Path(_)
            | NixValue::AttrSet { .. }
            | NixValue::LaidOutAttrSet { .. }
            | NixValue::List(_)
            | NixValue::LaidOutList { .. }
    );
    if base_is_primary {
        write_value(out, base, level, 0);
    } else {
        out.push('(');
        write_value(out, base, level, u8::MAX);
        out.push(')');
    }
    if !path.0.is_empty() {
        out.push('.');
        write_attr_path(out, path);
    }
    if let Some(d) = default {
        out.push_str(" or ");
        write_operand(out, d, level);
    }
}

/// A value in a position that binds tighter than application (a list
/// element, an `or` default): atoms, strings, collections and plain
/// selects bare, anything else parenthesized, so `[ (f x) y ]` and
/// `a.b or (f y)` keep their meaning.
fn write_operand(out: &mut String, v: &NixValue, level: usize) {
    let bare = is_atomic(v)
        || matches!(
            v,
            NixValue::Select { default: None, .. }
                | NixValue::InterpolatedStr(_)
                | NixValue::IndentedStr(_)
                | NixValue::IndentedInterpolatedStr(_)
                | NixValue::List(_)
                | NixValue::LaidOutList { .. }
                | NixValue::AttrSet { .. }
                | NixValue::LaidOutAttrSet { .. }
                | NixValue::Raw(_)
        );
    if bare {
        write_value(out, v, level, 0);
    } else {
        out.push('(');
        write_value(out, v, level, u8::MAX);
        out.push(')');
    }
}

fn write_inline_attrset(out: &mut String, entries: &[AttrSetEntry], level: usize) {
    out.push('{');
    for entry in entries {
        write_inline_entry(out, entry, level);
    }
    out.push_str(" }");
}

fn write_inline_entry(out: &mut String, entry: &AttrSetEntry, level: usize) {
    match entry {
        AttrSetEntry::Blank => {}
        AttrSetEntry::Comment(c) => {
            out.push(' ');
            write_inline_comment(out, c);
        }
        AttrSetEntry::Line { entries, comment } => {
            for e in entries {
                write_inline_entry(out, e, level);
            }
            if let Some(c) = comment {
                out.push(' ');
                write_inline_comment(out, c);
            }
        }
        AttrSetEntry::KeyValue { .. } | AttrSetEntry::Inherit { .. } => {
            out.push(' ');
            write_attrset_entry(out, entry, level);
        }
    }
}

fn write_block_attrset(out: &mut String, entries: &[AttrSetEntry], level: usize) {
    out.push('{');
    let inner = indent_str(level + 1);
    for entry in entries {
        out.push('\n');
        // A blank line carries no indentation.
        if matches!(entry, AttrSetEntry::Blank) {
            continue;
        }
        out.push_str(&inner);
        write_attrset_entry(out, entry, level + 1);
    }
    out.push('\n');
    out.push_str(&indent_str(level));
    out.push('}');
}

fn write_inline_list(out: &mut String, items: &[NixValue], level: usize) {
    if items.is_empty() {
        out.push_str("[ ]");
        return;
    }
    out.push_str("[ ");
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        write_operand(out, item, level);
    }
    out.push_str(" ]");
}

fn write_block_list(out: &mut String, items: &[NixValue], level: usize) {
    out.push('[');
    let inner = indent_str(level + 1);
    for item in items {
        out.push('\n');
        out.push_str(&inner);
        write_operand(out, item, level + 1);
    }
    out.push('\n');
    out.push_str(&indent_str(level));
    out.push(']');
}

fn write_quoted_str(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '$' => out.push_str("\\$"),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn write_indented_str(out: &mut String, lines: &[String], level: usize) {
    out.push_str("''");
    let inner_indent = indent_str(level + 1);
    for line in lines {
        out.push('\n');
        out.push_str(&inner_indent);
        out.push_str(line);
    }
    out.push('\n');
    out.push_str(&indent_str(level));
    out.push_str("''");
}

fn write_interpolated_str(out: &mut String, parts: &[StrPart], level: usize) {
    out.push('"');
    for part in parts {
        match part {
            StrPart::Literal(s) => {
                for c in s.chars() {
                    match c {
                        '"' => out.push_str("\\\""),
                        '\\' => out.push_str("\\\\"),
                        '\n' => out.push_str("\\n"),
                        '$' => out.push_str("\\$"),
                        c => out.push(c),
                    }
                }
            }
            StrPart::Interp(v) => {
                out.push_str("${");
                write_value(out, v, level, u8::MAX);
                out.push('}');
            }
        }
    }
    out.push('"');
}

fn write_list(out: &mut String, items: &[NixValue], level: usize) {
    if items.is_empty() {
        out.push_str("[ ]");
        return;
    }
    if items.iter().all(is_atomic) && items.len() <= 6 {
        out.push_str("[ ");
        for (i, item) in items.iter().enumerate() {
            if i > 0 {
                out.push(' ');
            }
            write_value(out, item, level, 0);
        }
        out.push_str(" ]");
        return;
    }
    out.push('[');
    let inner = indent_str(level + 1);
    for item in items {
        out.push('\n');
        out.push_str(&inner);
        write_value(out, item, level + 1, u8::MAX);
    }
    out.push('\n');
    out.push_str(&indent_str(level));
    out.push(']');
}

fn write_attrset(out: &mut String, recursive: bool, entries: &[AttrSetEntry], level: usize) {
    if recursive {
        out.push_str("rec ");
    }
    if entries.is_empty() {
        out.push_str("{ }");
        return;
    }
    write_block_attrset(out, entries, level);
}

fn write_attrset_entry(out: &mut String, entry: &AttrSetEntry, level: usize) {
    match entry {
        AttrSetEntry::KeyValue { key, value } => {
            write_attr_path(out, key);
            out.push_str(" = ");
            write_value(out, value, level, u8::MAX);
            out.push(';');
        }
        AttrSetEntry::Inherit { from, names } => {
            out.push_str("inherit");
            if let Some(f) = from {
                out.push_str(" (");
                write_value(out, f, level, u8::MAX);
                out.push(')');
            }
            for n in names {
                out.push(' ');
                out.push_str(n);
            }
            out.push(';');
        }
        AttrSetEntry::Comment(c) => write_comment(out, c),
        // A block Blank is written by `write_block_attrset`; one nested
        // in a `Line` has nothing to add.
        AttrSetEntry::Blank => {}
        AttrSetEntry::Line { entries, comment } => {
            let mut first = true;
            for e in entries {
                if matches!(e, AttrSetEntry::Blank) {
                    continue;
                }
                if !first {
                    out.push(' ');
                }
                first = false;
                match e {
                    // A `#` mid-line would swallow what follows it.
                    AttrSetEntry::Comment(c) => write_inline_comment(out, c),
                    _ => write_attrset_entry(out, e, level),
                }
            }
            if let Some(c) = comment {
                if !first {
                    out.push(' ');
                }
                write_comment(out, c);
            }
        }
    }
}

fn write_attr_path(out: &mut String, path: &AttrPath) {
    for (i, k) in path.0.iter().enumerate() {
        if i > 0 {
            out.push('.');
        }
        match k {
            AttrKey::Ident(s) => out.push_str(s),
            AttrKey::Str(s) => write_quoted_str(out, s),
            AttrKey::Interp(v) => {
                out.push_str("${");
                write_value(out, v, 0, u8::MAX);
                out.push('}');
            }
        }
    }
}

fn write_lambda(out: &mut String, params: &LambdaParams, body: &NixValue, level: usize) {
    write_lambda_params(out, params, level);
    out.push_str(": ");
    write_value(out, body, level, u8::MAX);
}

fn write_lambda_params(out: &mut String, params: &LambdaParams, level: usize) {
    match params {
        LambdaParams::Single(name) => out.push_str(name),
        LambdaParams::Destructured {
            fields,
            ellipsis,
            binding,
        } => {
            if let Some(b) = binding {
                out.push_str(b);
                out.push_str(" @ ");
            }
            out.push_str("{ ");
            for (i, f) in fields.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_param_field(out, f, level);
            }
            if *ellipsis {
                if !fields.is_empty() {
                    out.push_str(", ");
                }
                out.push_str("...");
            }
            out.push_str(" }");
        }
    }
}

fn write_param_field(out: &mut String, f: &ParamField, level: usize) {
    out.push_str(&f.name);
    if let Some(d) = &f.default {
        out.push_str(" ? ");
        write_value(out, d, level, u8::MAX);
    }
}

fn write_apply(
    out: &mut String,
    func: &NixValue,
    args: &[NixValue],
    level: usize,
    parent_prec: u8,
) {
    // Application binds tighter than most things; parenthesize when
    // parent has precedence < 5 (multiplicative/additive).
    let needs_parens = parent_prec < 5;
    if needs_parens {
        out.push('(');
    }
    write_value(out, func, level, 0);
    for arg in args {
        out.push(' ');
        match arg {
            NixValue::Apply { .. }
            | NixValue::BinOp { .. }
            | NixValue::Lambda { .. }
            | NixValue::Let { .. }
            | NixValue::If { .. }
            | NixValue::With { .. } => {
                out.push('(');
                write_value(out, arg, level, u8::MAX);
                out.push(')');
            }
            _ => write_value(out, arg, level, 0),
        }
    }
    if needs_parens {
        out.push(')');
    }
}

fn write_let(out: &mut String, bindings: &[LetBinding], body: &NixValue, level: usize) {
    write_let_head(out, bindings, level);
    out.push_str("in\n");
    out.push_str(&indent_str(level));
    write_value(out, body, level, u8::MAX);
}

/// `let`, one binding per line, then the indent `in` sits at. A blank
/// line carries no indentation.
fn write_let_head(out: &mut String, bindings: &[LetBinding], level: usize) {
    out.push_str("let\n");
    let inner = indent_str(level + 1);
    for b in bindings {
        if !matches!(b, LetBinding::Blank) {
            out.push_str(&inner);
            write_let_binding(out, b, level + 1);
        }
        out.push('\n');
    }
    out.push_str(&indent_str(level));
}

fn write_let_binding(out: &mut String, b: &LetBinding, level: usize) {
    match b {
        LetBinding::Bind { name, value } => {
            out.push_str(name);
            out.push_str(" = ");
            write_value(out, value, level, u8::MAX);
            out.push(';');
        }
        LetBinding::Inherit { from, names } => {
            out.push_str("inherit");
            if let Some(f) = from {
                out.push_str(" (");
                write_value(out, f, level, u8::MAX);
                out.push(')');
            }
            for n in names {
                out.push(' ');
                out.push_str(n);
            }
            out.push(';');
        }
        LetBinding::Comment(c) => write_comment(out, c),
        LetBinding::Blank => {}
    }
}

fn write_binop(
    out: &mut String,
    op: NixBinOp,
    left: &NixValue,
    right: &NixValue,
    level: usize,
    parent_prec: u8,
) {
    let my_prec = op.precedence();
    let needs_parens = my_prec > parent_prec;
    if needs_parens {
        out.push('(');
    }
    write_value(out, left, level, my_prec);
    out.push(' ');
    out.push_str(op.as_str());
    out.push(' ');
    write_value(out, right, level, my_prec);
    if needs_parens {
        out.push(')');
    }
}

fn is_atomic(v: &NixValue) -> bool {
    matches!(
        v,
        NixValue::Null
            | NixValue::Bool(_)
            | NixValue::Int(_)
            | NixValue::Float(_)
            | NixValue::Str(_)
            | NixValue::Path(_)
            | NixValue::Ident(_)
            | NixValue::AttrPath(_)
    )
}

#[cfg(test)]
mod layout_tests {
    use crate::ast::{
        AttrKey, AttrSetEntry, CollectionLayout, LambdaLayout, LetBinding, LetLayout, NixValue,
        StrPart, bind, destructured, entry, inherit, str_entry,
    };
    use crate::render::{render, render_file};

    fn s(x: &str) -> NixValue {
        NixValue::str(x)
    }

    #[test]
    fn select_interpolates_and_parenthesizes_an_applied_default() {
        let v = NixValue::select(
            NixValue::ident("sources"),
            [
                AttrKey::Interp(NixValue::ident("name")),
                AttrKey::Ident("meta".into()),
            ],
        );
        assert_eq!(render(&v), "sources.${name}.meta");

        let v = NixValue::select_or(
            NixValue::ident("sources"),
            [AttrKey::Interp(NixValue::ident("system"))],
            NixValue::apply(NixValue::ident("throw"), [s("no")]),
        );
        assert_eq!(render(&v), r#"sources.${system} or (throw "no")"#);

        let v = NixValue::select_or(
            NixValue::ident("x"),
            [AttrKey::Ident("a".into())],
            NixValue::Null,
        );
        assert_eq!(render(&v), "x.a or null");
    }

    #[test]
    fn commented_header_then_function_with_body_on_next_line() {
        let v = NixValue::commented(
            ["line one", "", "line two"],
            NixValue::lambda_laid_out(
                LambdaLayout::BodyOnNextLine,
                destructured(["pkgs", "sources"], false),
                NixValue::attrset_laid_out(CollectionLayout::Block, []),
            ),
        );
        assert_eq!(
            render_file(&v),
            "# line one\n#\n# line two\n{ pkgs, sources }:\n{\n}\n"
        );
    }

    #[test]
    fn block_entries_carry_comments_blank_lines_and_shared_lines() {
        let v = NixValue::attrset_laid_out(
            CollectionLayout::Block,
            [
                AttrSetEntry::Comment("section".into()),
                AttrSetEntry::Blank,
                entry("a", NixValue::Int(1)),
                AttrSetEntry::Line {
                    entries: vec![entry("b", s("x")), entry("c", s("y"))],
                    comment: Some("why".into()),
                },
                AttrSetEntry::Line {
                    entries: vec![entry("d", NixValue::Int(2))],
                    comment: None,
                },
                AttrSetEntry::Blank,
            ],
        );
        assert_eq!(
            render(&v),
            "{\n  # section\n\n  a = 1;\n  b = \"x\"; c = \"y\"; # why\n  d = 2;\n\n}"
        );
    }

    #[test]
    fn inline_attrset_never_emits_a_line_comment() {
        let v = NixValue::attrset_laid_out(
            CollectionLayout::Inline,
            [
                str_entry("aarch64-darwin", s("u")),
                inherit(Some(NixValue::ident("src")), ["url", "hash"]),
                AttrSetEntry::Blank,
                AttrSetEntry::Comment("note".into()),
            ],
        );
        assert_eq!(
            render(&v),
            r#"{ "aarch64-darwin" = "u"; inherit (src) url hash; /* note */ }"#
        );
        assert_eq!(
            render(&NixValue::attrset_laid_out(CollectionLayout::Inline, [])),
            "{ }"
        );
    }

    #[test]
    fn lists_honour_an_explicit_layout() {
        let many = (0..8).map(|i| s(&i.to_string()));
        let inline = NixValue::list_laid_out(CollectionLayout::Inline, many.clone());
        assert_eq!(render(&inline), r#"[ "0" "1" "2" "3" "4" "5" "6" "7" ]"#);
        let block = NixValue::list_laid_out(CollectionLayout::Block, [s("a")]);
        assert_eq!(render(&block), "[\n  \"a\"\n]");
        let empty = NixValue::list_laid_out(CollectionLayout::Block, []);
        assert_eq!(render(&empty), "[\n]");
        let applied = NixValue::list_laid_out(
            CollectionLayout::Inline,
            [NixValue::apply(
                NixValue::ident("f"),
                [NixValue::ident("x")],
            )],
        );
        assert_eq!(render(&applied), "[ (f x) ]");
    }

    #[test]
    fn let_body_after_in_with_comments_and_blank_bindings() {
        let v = NixValue::let_laid_out(
            LetLayout::BodyAfterIn,
            [
                LetBinding::Comment("helper".into()),
                bind("v", NixValue::Int(1)),
                LetBinding::Blank,
                bind("w", NixValue::Int(2)),
            ],
            NixValue::attrset_laid_out(CollectionLayout::Block, [entry("x", NixValue::ident("v"))]),
        );
        assert_eq!(
            render(&v),
            "let\n  # helper\n  v = 1;\n\n  w = 2;\nin {\n  x = v;\n}"
        );
    }

    #[test]
    fn indented_interpolated_string_escapes_only_literals() {
        let v = NixValue::IndentedInterpolatedStr(vec![
            vec![
                StrPart::Literal("mv $out/a ''b'' ${raw} _v".into()),
                StrPart::Interp(NixValue::apply(NixValue::ident("v"), [s("k")])),
            ],
            vec![],
            vec![StrPart::Literal("end".into())],
        ]);
        assert_eq!(
            render(&v),
            "''\n  mv $out/a '''b''' ''${raw} _v${v \"k\"}\n\n  end\n''"
        );
    }

    #[test]
    fn canonical_constructs_are_unchanged_by_the_layout_siblings() {
        let v = NixValue::Let {
            bindings: vec![bind("a", NixValue::Int(1))],
            body: Box::new(NixValue::ident("a")),
        };
        assert_eq!(render(&v), "let\n  a = 1;\nin\na");
        let f = NixValue::Lambda {
            params: destructured(["pkgs"], true),
            body: Box::new(NixValue::ident("pkgs")),
        };
        assert_eq!(render(&f), "{ pkgs, ... }: pkgs");
    }
}
