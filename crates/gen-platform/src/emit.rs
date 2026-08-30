//! Emit substrate-side `quirk-apply.nix` skeletons from typed
//! `TypedDispatcher` reflection. Closes the loop typed Rust enum
//! → typed Nix dispatch arm without the operator hand-writing the
//! mapping table.
//!
//! Pattern: given a `DispatcherEntry` (catalog) or any
//! `TypedDispatcher` impl, produce the Nix `helpers = { ... }`
//! table skeleton that the substrate's
//! `mk-typed-dispatcher.nix` combinator consumes. The skeleton
//! has typed stub bodies (`{}`) per variant; the operator fills in
//! the apply functions per ecosystem.
//!
//! See `theory/TYPED-ABSORPTION.md` §IV.2 — this is the
//! "mechanical absorption" step: adding a new ecosystem becomes
//! "declare the Rust enum, emit the Nix skeleton, fill in
//! helpers." No dispatch fold reimplementation. No Nix parser
//! gymnastics.

use crate::catalog::DispatcherEntry;

/// Render a `quirk-apply.nix` skeleton for a dispatcher catalog
/// entry. Output is a complete, syntactically-valid Nix file
/// (verified by `nix-instantiate --eval --strict`) consuming
/// `../shared/mk-typed-dispatcher.nix` with one helpers arm per
/// variant kind.
///
/// Field bindings:
/// - Unit variants → `_: {}` body.
/// - Single-field variants → `<field>Apply quirk.<field>` body.
/// - Multi-field variants → `<verb>Apply { inherit (quirk) <fields>...; }` body.
///
/// The class-helper function names use camelCase derived from the
/// kebab-case kind (e.g. `pin-toolchain` → `pinToolchainApply`).
#[must_use]
pub fn to_helpers_skeleton(entry: &DispatcherEntry) -> String {
    let variant_fields = (entry.variant_fields)();
    let mut out = String::with_capacity(512);
    out.push_str(&format!(
        "# quirk-apply.nix — mechanically emitted skeleton for the\n"
    ));
    out.push_str(&format!(
        "# `{}` dispatcher (gen_platform::DispatcherCatalog).\n",
        entry.label
    ));
    out.push_str("# Each helpers arm calls a class-helper function — fill the\n");
    out.push_str("# function bodies in the `let` block below to drive the\n");
    out.push_str("# matching builder.\n");
    out.push_str("#\n");
    out.push_str("# Refresh via:\n#   gen dispatchers helpers-skeleton \\\n#     --label ");
    out.push_str(entry.label);
    out.push_str(" --format nix\n");
    out.push_str("{ lib }:\nlet\n");

    // One class-helper stub per variant. Body is `_: {}` (no-op);
    // the operator replaces with the real apply logic.
    for (kind, fields) in &variant_fields {
        let helper_name = format!("{}Apply", kebab_to_camel(kind));
        if fields.is_empty() {
            out.push_str(&format!("  # Unit variant — no payload.\n"));
            out.push_str(&format!("  {helper_name} = _: _: {{}};\n\n"));
        } else if fields.len() == 1 {
            let field = &fields[0];
            out.push_str(&format!(
                "  # Single-field variant — receives the field's value.\n"
            ));
            out.push_str(&format!("  {helper_name} = {field}: _: {{}};\n\n"));
        } else {
            let inherit_str = fields.join(" ");
            out.push_str(&format!(
                "  # Multi-field variant — destructure the typed payload.\n"
            ));
            out.push_str(&format!(
                "  {helper_name} = {{ {} }}: _: {{}};\n\n",
                inherit_str.replace(' ', ", ")
            ));
        }
    }

    out.push_str("in\nimport ../shared/mk-typed-dispatcher.nix {\n");
    out.push_str("  inherit lib;\n");
    out.push_str("  helpers = {\n");

    for (kind, fields) in &variant_fields {
        let helper_name = format!("{}Apply", kebab_to_camel(kind));
        if fields.is_empty() {
            out.push_str(&format!("    \"{kind}\" = quirk: {helper_name} quirk;\n"));
        } else if fields.len() == 1 {
            let field = &fields[0];
            out.push_str(&format!(
                "    \"{kind}\" = quirk: {helper_name} quirk.{field};\n"
            ));
        } else {
            let inherit_str = fields.join(" ");
            out.push_str(&format!("    \"{kind}\" = quirk: {helper_name} {{\n"));
            out.push_str(&format!("      inherit (quirk) {inherit_str};\n"));
            out.push_str("    };\n");
        }
    }

    out.push_str("  };\n}\n");
    out
}

/// kebab-case → camelCase. `pin-toolchain` → `pinToolchain`.
fn kebab_to_camel(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut upper_next = false;
    for ch in s.chars() {
        if ch == '-' {
            upper_next = true;
        } else if upper_next {
            for u in ch.to_uppercase() {
                out.push(u);
            }
            upper_next = false;
        } else {
            out.push(ch);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kebab_to_camel_handles_simple_cases() {
        assert_eq!(kebab_to_camel("force-cfg"), "forceCfg");
        assert_eq!(kebab_to_camel("substitute-source"), "substituteSource");
        assert_eq!(kebab_to_camel("ldflag"), "ldflag");
        assert_eq!(kebab_to_camel("pin-toolchain"), "pinToolchain");
        assert_eq!(
            kebab_to_camel("fold-normal-into-build"),
            "foldNormalIntoBuild"
        );
    }
}
