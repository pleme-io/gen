//! Canonical pretty-printer for the Starlark emission AST.

use crate::ast::{KwArg, StarlarkStmt, StarlarkValue};

/// Render a sequence of statements to a BUILD/MODULE file body.
pub fn render_file(stmts: &[StarlarkStmt]) -> String {
    let mut out = String::new();
    for (i, stmt) in stmts.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        render_stmt(&mut out, stmt);
        out.push('\n');
    }
    out
}

fn render_stmt(out: &mut String, stmt: &StarlarkStmt) {
    match stmt {
        StarlarkStmt::Load { module, symbols } => {
            out.push_str("load(\"");
            out.push_str(module);
            out.push('"');
            for s in symbols {
                out.push_str(", \"");
                out.push_str(s);
                out.push('"');
            }
            out.push(')');
        }
        StarlarkStmt::Call { func, args } => {
            render_call(out, func, args, 0);
        }
        StarlarkStmt::Assign { name, value } => {
            out.push_str(name);
            out.push_str(" = ");
            render_value(out, value, 0);
        }
    }
}

fn render_call(out: &mut String, func: &str, args: &[KwArg], indent: usize) {
    out.push_str(func);
    out.push('(');
    if args.is_empty() {
        out.push(')');
        return;
    }
    let multi = args.len() > 2 || args_have_multiline(args);
    if multi {
        for arg in args {
            out.push('\n');
            push_indent(out, indent + 1);
            render_kwarg(out, arg, indent + 1);
            out.push(',');
        }
        out.push('\n');
        push_indent(out, indent);
    } else {
        for (i, arg) in args.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            render_kwarg(out, arg, indent);
        }
    }
    out.push(')');
}

fn render_kwarg(out: &mut String, arg: &KwArg, indent: usize) {
    match arg {
        KwArg::Positional(v) => render_value(out, v, indent),
        KwArg::Named { name, value } => {
            out.push_str(name);
            out.push_str(" = ");
            render_value(out, value, indent);
        }
    }
}

fn render_value(out: &mut String, v: &StarlarkValue, indent: usize) {
    match v {
        StarlarkValue::None => out.push_str("None"),
        StarlarkValue::Bool(true) => out.push_str("True"),
        StarlarkValue::Bool(false) => out.push_str("False"),
        StarlarkValue::Int(i) => out.push_str(&i.to_string()),
        StarlarkValue::Str(s) => {
            out.push('"');
            for c in s.chars() {
                match c {
                    '"' => out.push_str("\\\""),
                    '\\' => out.push_str("\\\\"),
                    '\n' => out.push_str("\\n"),
                    c => out.push(c),
                }
            }
            out.push('"');
        }
        StarlarkValue::Ident(s) => out.push_str(s),
        StarlarkValue::List(items) => render_list(out, items, indent),
        StarlarkValue::Dict(entries) => render_dict(out, entries, indent),
        StarlarkValue::Call { func, args } => render_call(out, func, args, indent),
    }
}

fn render_list(out: &mut String, items: &[StarlarkValue], indent: usize) {
    if items.is_empty() {
        out.push_str("[]");
        return;
    }
    let multi = items.len() > 3 || any_value_multiline(items);
    out.push('[');
    if multi {
        for v in items {
            out.push('\n');
            push_indent(out, indent + 1);
            render_value(out, v, indent + 1);
            out.push(',');
        }
        out.push('\n');
        push_indent(out, indent);
    } else {
        for (i, v) in items.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            render_value(out, v, indent);
        }
    }
    out.push(']');
}

fn render_dict(out: &mut String, entries: &[(String, StarlarkValue)], indent: usize) {
    if entries.is_empty() {
        out.push_str("{}");
        return;
    }
    out.push('{');
    for (k, v) in entries {
        out.push('\n');
        push_indent(out, indent + 1);
        out.push('"');
        out.push_str(k);
        out.push_str("\": ");
        render_value(out, v, indent + 1);
        out.push(',');
    }
    out.push('\n');
    push_indent(out, indent);
    out.push('}');
}

fn args_have_multiline(args: &[KwArg]) -> bool {
    args.iter().any(|a| match a {
        KwArg::Positional(v) | KwArg::Named { value: v, .. } => is_value_multiline(v),
    })
}

fn any_value_multiline(items: &[StarlarkValue]) -> bool {
    items.iter().any(is_value_multiline)
}

fn is_value_multiline(v: &StarlarkValue) -> bool {
    match v {
        StarlarkValue::List(items) => items.len() > 3 || any_value_multiline(items),
        StarlarkValue::Dict(_) => true,
        StarlarkValue::Call { args, .. } => args.len() > 2 || args_have_multiline(args),
        _ => false,
    }
}

fn push_indent(out: &mut String, level: usize) {
    for _ in 0..level {
        out.push_str("    ");
    }
}
