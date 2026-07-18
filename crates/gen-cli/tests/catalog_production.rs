//! Cross-adapter `DispatcherCatalog` sweep — proves every
//! production Quirk enum is registered into the fleet-wide
//! catalog via `gen_platform::register_dispatcher!`. Adding a
//! new ecosystem WITHOUT a register_dispatcher! call fails this
//! test, enforcing the "common language" rule across the 9
//! adapters.

// Pull every adapter into the link-time inventory.
use gen_ansible as _;
use gen_bundler as _;
use gen_cargo as _;
use gen_gomod as _;
use gen_helm as _;
use gen_npm as _;
use gen_pip as _;
use gen_poetry as _;
use gen_swift as _;

use gen_platform::catalog;

const PRODUCTION_LABELS: &[&str] = &[
    "gen.ansible.ansible-quirk",
    "gen.bundler.bundler-quirk",
    "gen.cargo.crate-quirk",
    "gen.gomod.gomod-quirk",
    "gen.helm.helm-quirk",
    "gen.npm.npm-quirk",
    "gen.pip.pip-quirk",
    "gen.poetry.poetry-quirk",
    "gen.swift.swift-quirk",
];

#[test]
fn every_adapter_registers_into_catalog() {
    let mut missing = Vec::new();
    for label in PRODUCTION_LABELS {
        if catalog::by_label(label).is_none() {
            missing.push(*label);
        }
    }
    assert!(
        missing.is_empty(),
        "9 production dispatchers must be registered; missing: {missing:?}"
    );
}

#[test]
fn catalog_has_at_least_nine_entries() {
    // ≥ 9 not == 9: gen-cli links extra crates that may register
    // dispatchers later (e.g. gen-config, gen-nix). The strict
    // "9 production are present" assertion is the test above.
    let all = catalog::registered();
    assert!(
        all.len() >= 9,
        "catalog should hold ≥ 9 production entries; got {}",
        all.len()
    );
}

#[test]
fn each_production_variant_count_matches_expected() {
    // Tracks the variant universe per ecosystem. Failing this
    // means a Rust enum gained/lost a variant — refresh the
    // substrate's lib/build/shared/dispatcher-catalog.json + add
    // the matching helpers arm in substrate/lib/build/<eco>/quirk-apply.nix.
    let expected: &[(&str, usize)] = &[
        ("gen.cargo.crate-quirk", 4),
        ("gen.npm.npm-quirk", 5),
        ("gen.bundler.bundler-quirk", 5),
        ("gen.helm.helm-quirk", 4),
        ("gen.pip.pip-quirk", 4),
        ("gen.poetry.poetry-quirk", 4),
        ("gen.gomod.gomod-quirk", 5),
        ("gen.ansible.ansible-quirk", 4),
        ("gen.swift.swift-quirk", 4),
    ];
    for (label, count) in expected {
        let entry =
            catalog::by_label(label).unwrap_or_else(|| panic!("missing label {label}"));
        let got = (entry.variant_count)();
        assert_eq!(
            got, *count,
            "{label} variant_count drift: expected {count}, got {got}"
        );
    }
}

#[test]
fn variant_kinds_are_globally_unique_per_dispatcher() {
    // Within one dispatcher, every kind tag must be unique
    // (serde would have failed earlier, but this is the explicit
    // assertion).
    for entry in catalog::registered() {
        let kinds = (entry.variant_kinds)();
        let mut seen = Vec::with_capacity(kinds.len());
        for k in &kinds {
            assert!(
                !seen.contains(k),
                "{}: duplicate kind {k}",
                entry.label
            );
            seen.push(*k);
        }
    }
}
