//! Cross-adapter `TypedDispatcher` reflection sweep.
//!
//! Every adapter's Quirk enum carries `#[derive(TypedDispatcher)]`
//! (P0/P1 of the substrate-wide quirk-applier normalization per
//! `theory/QUIRK-APPLIER.md` §IV-bis). This test asserts the
//! reflection is shaped consistently across all 9 ecosystems:
//!
//! - every variant_kinds() entry is kebab-case (matches the serde
//!   tag the runtime emits);
//! - variant_fields()'s tag keys equal variant_kinds() (same order,
//!   same set);
//! - variant_count() agrees with variant_kinds().len();
//! - no two variants in the same enum share a tag (uniqueness).
//!
//! Adding a new ecosystem with a TypedDispatcher-deriving enum is
//! one row in the table below — the same shape every existing
//! adapter satisfies mechanically.

use gen_types::TypedDispatcher;

fn is_kebab(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !s.starts_with('-')
        && !s.ends_with('-')
}

fn check<T: TypedDispatcher>(name: &'static str) {
    let kinds = T::variant_kinds();
    let fields = T::variant_fields();

    assert!(!kinds.is_empty(), "{name}: no variants declared");
    assert_eq!(
        kinds.len(),
        T::variant_count(),
        "{name}: variant_count() != variant_kinds().len()"
    );
    assert_eq!(
        kinds.len(),
        fields.len(),
        "{name}: variant_kinds() and variant_fields() length mismatch"
    );

    for k in &kinds {
        assert!(is_kebab(k), "{name}: variant kind '{k}' is not kebab-case");
    }
    let fields_kinds: Vec<&'static str> = fields.iter().map(|(k, _)| *k).collect();
    assert_eq!(
        kinds, fields_kinds,
        "{name}: variant_fields() keys must match variant_kinds() exactly"
    );

    let mut seen: Vec<&&'static str> = Vec::with_capacity(kinds.len());
    for k in &kinds {
        assert!(
            !seen.contains(&k),
            "{name}: duplicate variant kind '{k}'"
        );
        seen.push(k);
    }
}

#[test]
fn cargo_crate_quirk_reflection() {
    check::<gen_cargo::quirks::CrateQuirk>("CrateQuirk");
}

#[test]
fn npm_quirk_reflection() {
    check::<gen_npm::quirks::NpmQuirk>("NpmQuirk");
}

#[test]
fn bundler_quirk_reflection() {
    check::<gen_bundler::quirks::BundlerQuirk>("BundlerQuirk");
}

#[test]
fn helm_quirk_reflection() {
    check::<gen_helm::quirks::HelmQuirk>("HelmQuirk");
}

#[test]
fn pip_quirk_reflection() {
    check::<gen_pip::quirks::PipQuirk>("PipQuirk");
}

#[test]
fn poetry_quirk_reflection() {
    check::<gen_poetry::quirks::PoetryQuirk>("PoetryQuirk");
}

#[test]
fn gomod_quirk_reflection() {
    check::<gen_gomod::quirks::GomodQuirk>("GomodQuirk");
}

#[test]
fn ansible_quirk_reflection() {
    check::<gen_ansible::quirks::AnsibleQuirk>("AnsibleQuirk");
}

#[test]
fn swift_quirk_reflection() {
    check::<gen_swift::quirks::SwiftQuirk>("SwiftQuirk");
}

#[test]
fn variant_kind_counts() {
    // Tracks the substrate's typed-dispatcher surface area.
    // Update when an enum gains/loses a variant — the test fails
    // and the substrate's Nix helpers table needs the matching arm.
    assert_eq!(gen_cargo::quirks::CrateQuirk::variant_count(), 3);
    assert_eq!(gen_npm::quirks::NpmQuirk::variant_count(), 5);
    assert_eq!(gen_bundler::quirks::BundlerQuirk::variant_count(), 5);
    assert_eq!(gen_helm::quirks::HelmQuirk::variant_count(), 4);
    assert_eq!(gen_pip::quirks::PipQuirk::variant_count(), 4);
    assert_eq!(gen_poetry::quirks::PoetryQuirk::variant_count(), 4);
    assert_eq!(gen_gomod::quirks::GomodQuirk::variant_count(), 5);
    assert_eq!(gen_ansible::quirks::AnsibleQuirk::variant_count(), 4);
    assert_eq!(gen_swift::quirks::SwiftQuirk::variant_count(), 4);
}
