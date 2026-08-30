//! The PDC verification MATRIX — CLOSED-LOOP MASS-SYNTHESIS rule 1.
//!
//! ONE matrix test with ONE row per ecosystem variant, exercising each
//! against the per-dependency-caching INVARIANT:
//!
//!   > a shared dependency is built ONCE and reused across ≥2 consumers,
//!   > every buildable node carries a per-dependency content address, every
//!   > edge resolves, and the schema is fresh.
//!
//! The matrix FAILS THE BUILD when:
//!   - a Shipped/M1 variant's fixture violates the invariant
//!     (`every_shipped_and_m1_variant_satisfies_the_pdc_invariant`);
//!   - a new catalogued ecosystem lands without a matrix row, or a matrix
//!     row names an ecosystem absent from the catalog
//!     (`matrix_covers_every_catalogued_ecosystem` — the forcing function);
//!   - a Design variant is silently claimed conformant, or a Design gap
//!     silently closes (`design_variants_are_honestly_non_conformant`).
//!
//! Failures aggregate BEFORE the assert — one run reports every broken row,
//! not just the first (CLOSED-LOOP MASS-SYNTHESIS rule 1).

use gen_pdc::catalog::{CATALOG, Maturity};
use gen_pdc::check::{PdcCheckOutcome, check};
use gen_pdc::fixture;
use gen_pdc::spec::PdcSpec;

/// One row of the matrix: an ecosystem name + the fixture that stands in
/// for its projected spec + whether it is EXPECTED to satisfy the full
/// per-dependency invariant today (Shipped/M1 = yes; Design = no, honestly).
struct MatrixRow {
    ecosystem: &'static str,
    fixture: fn() -> PdcSpec,
    /// The maturity the catalog claims — the matrix cross-checks it.
    expected_maturity: Maturity,
}

/// The matrix. ONE row per ecosystem in the catalog. Adding a catalog entry
/// without adding a row here fails `matrix_covers_every_catalogued_ecosystem`.
const MATRIX: &[MatrixRow] = &[
    MatrixRow {
        ecosystem: "cargo",
        fixture: fixture::cargo_fixture,
        expected_maturity: Maturity::Shipped,
    },
    MatrixRow {
        ecosystem: "gomod",
        fixture: fixture::gomod_fixture,
        expected_maturity: Maturity::M1,
    },
    MatrixRow {
        ecosystem: "npm",
        fixture: fixture::npm_design_fixture,
        expected_maturity: Maturity::Design,
    },
    MatrixRow {
        ecosystem: "pip",
        fixture: fixture::pip_design_fixture,
        expected_maturity: Maturity::Design,
    },
];

/// Run one row against the universal checker, using the variant's admitted
/// source classes (the clause-5 partition) from the catalog.
fn run_row(row: &MatrixRow) -> PdcCheckOutcome {
    let v = gen_pdc::catalog::variant(row.ecosystem)
        .unwrap_or_else(|| panic!("matrix row {} not in catalog", row.ecosystem));
    let spec = (row.fixture)();
    check(&spec, v.admitted_classes)
}

#[test]
fn matrix_covers_every_catalogued_ecosystem() {
    // The forcing function (CATALOG REFLECTION + CLOSED-LOOP rule 1): the
    // matrix rows and the catalog entries are the SAME set. A new ecosystem
    // cannot ship a catalog entry without a matrix row, and vice versa.
    let mut catalog_names: Vec<&str> = CATALOG.iter().map(|v| v.name).collect();
    let mut matrix_names: Vec<&str> = MATRIX.iter().map(|r| r.ecosystem).collect();
    catalog_names.sort_unstable();
    matrix_names.sort_unstable();
    assert_eq!(
        catalog_names, matrix_names,
        "PDC matrix ⇄ catalog drift: every catalogued ecosystem needs exactly one matrix row"
    );
    // And the matrix must be at least as large as the current fleet.
    assert!(
        MATRIX.len() >= 4,
        "matrix regressed below the four known ecosystems"
    );
}

#[test]
fn matrix_maturity_agrees_with_catalog() {
    // Each row's expected_maturity must match the catalog — the matrix is a
    // second witness of the ledger, so a maturity edit in one place without
    // the other fails here.
    let mut failures = Vec::new();
    for row in MATRIX {
        let v = gen_pdc::catalog::variant(row.ecosystem).unwrap();
        if v.maturity != row.expected_maturity {
            failures.push(format!(
                "{}: matrix says {:?}, catalog says {:?}",
                row.ecosystem, row.expected_maturity, v.maturity
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "maturity drift:\n  - {}",
        failures.join("\n  - ")
    );
}

#[test]
fn every_shipped_and_m1_variant_satisfies_the_pdc_invariant() {
    // THE core matrix assertion. For every Shipped/M1 ecosystem, its fixture
    // — a shared dependency reused by ≥2 consumers — must:
    //   (a) be PDC-valid (no clause violation), AND
    //   (b) OBSERVE dedup (the shared node built once, served to ≥2
    //       dependents) — the "compiled once, linked into every binary" win.
    // Failures aggregate; one run reports every broken variant.
    let mut failures: Vec<String> = Vec::new();

    for row in MATRIX {
        if !matches!(row.expected_maturity, Maturity::Shipped | Maturity::M1) {
            continue;
        }
        let outcome = run_row(row);
        if !outcome.is_valid() {
            failures.push(format!(
                "{}: PDC-invalid — {:?}",
                row.ecosystem, outcome.violations
            ));
        }
        if !outcome.observed_dedup() {
            failures.push(format!(
                "{}: shared dependency was NOT deduped (computed={}, served={}) — \
                 the per-dependency caching win is absent",
                row.ecosystem, outcome.nodes_computed, outcome.addr_requests_served
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "{} shipped/M1 variant(s) failed the PDC invariant:\n  - {}",
        failures.len(),
        failures.join("\n  - ")
    );
}

#[test]
fn design_variants_are_honestly_non_conformant() {
    // The tier-honest half: a Design ecosystem's fixture MUST fail the
    // per-dependency invariant (npm's whole-tree hash gap; pip's empty
    // border). If a Design fixture starts PASSING, the variant has advanced
    // and its catalog maturity must be promoted DELIBERATELY — this test
    // forces that decision instead of letting a gap silently close.
    let mut wrongly_conformant = Vec::new();
    for row in MATRIX {
        if row.expected_maturity != Maturity::Design {
            continue;
        }
        let outcome = run_row(row);
        // "Conformant" for a design row would be: valid AND dedup observed.
        if outcome.is_valid() && outcome.observed_dedup() {
            wrongly_conformant.push(row.ecosystem);
        }
    }
    assert!(
        wrongly_conformant.is_empty(),
        "design variant(s) now satisfy the PDC invariant — promote their maturity deliberately: {wrongly_conformant:?}"
    );
}

#[test]
fn the_pdc_invariant_catches_a_broken_spec() {
    // Adversarial: a mutated fixture that breaks the invariant (a dangling
    // edge) must be caught — proves the matrix has teeth, not a vacuous pass.
    let mut spec = fixture::cargo_fixture();
    spec.nodes
        .get_mut("bin-a")
        .unwrap()
        .deps
        .push("ghost".to_string()); // ghost ∉ nodes
    let v = gen_pdc::catalog::variant("cargo").unwrap();
    let outcome = check(&spec, v.admitted_classes);
    assert!(
        !outcome.is_valid(),
        "a dangling edge must be a PDC violation (clause 2)"
    );
    assert!(
        outcome
            .violations
            .iter()
            .any(|x| matches!(x, gen_pdc::check::PdcViolation::UnresolvedEdge { .. })),
        "expected an UnresolvedEdge violation, got {:?}",
        outcome.violations
    );
}
