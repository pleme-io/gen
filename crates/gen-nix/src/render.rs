//! Canonical pretty-printer for [`NixValue`]. Deterministic +
//! idempotent + nixpkgs-style indent. The only function downstream
//! consumers call to turn a typed AST into Nix source.

use crate::ast::{
    AttrKey, AttrPath, AttrSetEntry, LambdaParams, LetBinding, NixBinOp, NixValue, ParamField,
    StrPart,
};

const INDENT_STEP: usize = 2;

/// Render `value` to canonical Nix source. Entry point.
pub fn render(value: &NixValue) -> String {
    let mut out = String::new();
    write_value(&mut out, value, 0, u8::MAX);
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
        NixValue::Raw(s) => out.push_str(s),
    }
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
    out.push('{');
    let inner = indent_str(level + 1);
    for entry in entries {
        out.push('\n');
        out.push_str(&inner);
        write_attrset_entry(out, entry, level + 1);
    }
    out.push('\n');
    out.push_str(&indent_str(level));
    out.push('}');
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
    match params {
        LambdaParams::Single(name) => {
            out.push_str(name);
            out.push_str(": ");
            write_value(out, body, level, u8::MAX);
        }
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
            out.push_str(" }: ");
            write_value(out, body, level, u8::MAX);
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
    out.push_str("let\n");
    let inner = indent_str(level + 1);
    for b in bindings {
        out.push_str(&inner);
        write_let_binding(out, b, level + 1);
        out.push('\n');
    }
    out.push_str(&indent_str(level));
    out.push_str("in\n");
    out.push_str(&indent_str(level));
    write_value(out, body, level, u8::MAX);
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
