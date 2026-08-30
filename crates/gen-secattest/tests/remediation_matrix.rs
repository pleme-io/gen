//! The security-REMEDIATION verification MATRIX — CLOSED-LOOP MASS-SYNTHESIS
//! rule 1 applied to the growable CVE→fix catalog (`theory/SECURITY-LAYER.md`
//! §10).
//!
//! ONE matrix with ONE row per known CVE finding, exercising:
//!   - **present** — the finding resolves to a [`RemediationEntry`] whose
//!     `cve_id` matches the typed `CveFinding::cve_id()` (the compile-time
//!     exhaustive match in `remediation.rs` is the FIRST forcing function —
//!     a `CveFinding` variant with no `cve_id()` match arm fails to compile;
//!     this matrix is the SECOND forcing function — a variant with no
//!     catalog row still compiles, so a runtime test is what catches it);
//!   - **named fix** — the entry's [`Fix`] is a real typed mechanism, not an
//!     empty/blank payload (exhaustive match over `Fix`, no wildcard arm —
//!     a new `Fix` variant with no arm here also fails to COMPILE);
//!   - **dependency-edge-mapped** — `applies_to.{repo,layer}` are both
//!     non-empty (a remediation with no named dependency edge is useless to
//!     a sibling consumer trying to find it);
//!   - **honestly graded** — a `Design` entry names ZERO verified consumers
//!     as evidence it does NOT yet claim (never round DOWN a claim either —
//!     `VerifiedOnce`/`PropagatedFleetWide` entries must actually list the
//!     consumers the maturity implies exist).
//!
//! The forcing functions:
//!   - `matrix_covers_every_known_finding` — the matrix rows, the catalog
//!     entries, AND `CveFinding::ALL` are the SAME set. A new finding cannot
//!     ship a catalog entry (or a matrix row) without the other two.
//!   - `every_row_has_a_present_named_and_mapped_fix` — the per-row law.
//!
//! Failures AGGREGATE before the assert — one run reports every broken row.

use gen_secattest::remediation::{
    CveFinding, Fix, REMEDIATION_CATALOG, RemediationMaturity, maturity_histogram, remediation_for,
    remediation_for_cve_id,
};

/// One row of the matrix: a known finding + its expected CVE id string (the
/// catalog cross-checks the fix/applies-to/maturity fields).
struct MatrixRow {
    finding: CveFinding,
    expected_cve_id: &'static str,
}

/// The matrix. ONE row per known finding. Adding a `CveFinding` variant (and
/// its catalog entry) without a row here fails `matrix_covers_every_known_finding`.
const MATRIX: &[MatrixRow] = &[MatrixRow {
    finding: CveFinding::Cve202639822GoStdlib,
    expected_cve_id: "CVE-2026-39822",
}];

#[test]
fn matrix_covers_every_known_finding() {
    // The forcing function: matrix rows, catalog entries, and CveFinding::ALL
    // are the SAME set — a new finding cannot ship any one of the three
    // without the other two landing in the same commit.
    let mut catalog_findings: Vec<CveFinding> =
        REMEDIATION_CATALOG.iter().map(|e| e.finding).collect();
    let mut matrix_findings: Vec<CveFinding> = MATRIX.iter().map(|r| r.finding).collect();
    let mut all_findings: Vec<CveFinding> = CveFinding::ALL.to_vec();
    catalog_findings.sort_unstable();
    matrix_findings.sort_unstable();
    all_findings.sort_unstable();

    assert_eq!(
        catalog_findings, matrix_findings,
        "remediation matrix ⇄ catalog drift: every catalogued finding needs exactly one matrix row"
    );
    assert_eq!(
        catalog_findings, all_findings,
        "remediation catalog ⇄ CveFinding::ALL drift: every known finding needs exactly one catalog entry"
    );
    assert!(
        MATRIX.len() >= 1,
        "matrix regressed below the one known seed finding"
    );
}

#[test]
fn every_row_has_a_present_named_and_mapped_fix() {
    // The per-row law: present (resolves + cve_id matches) + named fix
    // (typed, non-empty payload) + dependency-edge-mapped (applies_to is
    // fully named). Aggregates every failure before asserting.
    let mut failures: Vec<String> = Vec::new();

    for row in MATRIX {
        let Some(e) = remediation_for(row.finding) else {
            failures.push(format!("{:?}: no catalog entry (not present)", row.finding));
            continue;
        };

        // present: cve_id agrees with both the row and the typed finding.
        if e.cve_id != row.expected_cve_id {
            failures.push(format!(
                "{:?}: matrix expects cve_id {:?}, catalog has {:?}",
                row.finding, row.expected_cve_id, e.cve_id
            ));
        }
        if e.cve_id != row.finding.cve_id() {
            failures.push(format!(
                "{:?}: catalog cve_id {:?} disagrees with the typed finding's own cve_id() {:?}",
                row.finding,
                e.cve_id,
                row.finding.cve_id()
            ));
        }

        // named fix: a real typed mechanism, non-empty payload. Exhaustive
        // match, NO wildcard arm — a new Fix variant with no arm here fails
        // to COMPILE (the same forcing shape as CveFinding::cve_id()).
        let fix_is_named = match e.fix {
            Fix::VersionPin { fixed_version } => !fixed_version.is_empty(),
            Fix::NixOverlay { overlay_name } => !overlay_name.is_empty(),
            Fix::SourcePatch { patch_ref } => !patch_ref.is_empty(),
            Fix::FlakeInputRepin { input_name, to_rev } => {
                !input_name.is_empty() && !to_rev.is_empty()
            }
        };
        if !fix_is_named {
            failures.push(format!(
                "{:?}: fix mechanism has an empty payload (not named)",
                row.finding
            ));
        }

        // dependency-edge-mapped: applies_to is fully named.
        if e.applies_to.repo.is_empty() || e.applies_to.layer.is_empty() {
            failures.push(format!(
                "{:?}: applies_to is not fully named (repo={:?}, layer={:?})",
                row.finding, e.applies_to.repo, e.applies_to.layer
            ));
        }

        // honestly graded: verified_consumers count must meet (never exceed
        // in the "claims more than it has" direction) the maturity's floor.
        let floor = e.maturity.minimum_verified_consumers();
        if e.verified_consumers.len() < floor {
            failures.push(format!(
                "{:?}: maturity {:?} claims a floor of {} verified consumers but only has {}",
                row.finding,
                e.maturity,
                floor,
                e.verified_consumers.len()
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "{} remediation row(s) failed present+named-fix+mapped+honestly-graded:\n  - {}",
        failures.len(),
        failures.join("\n  - ")
    );
}

#[test]
fn a_cve_id_lookup_resolves_the_same_entry_as_a_finding_lookup() {
    // The build-time-gate-facing lookup path (a raw CVE id string) must
    // resolve to the SAME entry as the typed finding lookup — the two
    // vocabularies (typed CveFinding, raw string a scanner emits) must never
    // silently diverge.
    for row in MATRIX {
        let by_finding = remediation_for(row.finding).expect("finding lookup must resolve");
        let by_cve_id =
            remediation_for_cve_id(row.expected_cve_id).expect("cve_id lookup must resolve");
        assert_eq!(
            by_finding.finding, by_cve_id.finding,
            "finding-lookup and cve_id-lookup diverged for {:?}",
            row.finding
        );
    }
    assert!(
        remediation_for_cve_id("CVE-0000-00000").is_none(),
        "an unknown cve_id must resolve to nothing, never a phantom entry"
    );
}

#[test]
fn no_entry_is_shipped_claiming_shipped_confirmation_it_has_not_earned() {
    // Belt-and-braces with remediation.rs's own no_entry_claims_more_
    // verification_than_it_has_evidence_for: the histogram partitions the
    // catalog exactly, and the seed entry's maturity is the honest snapshot
    // (Design — landed, not yet trivy-re-scan-confirmed).
    let (design, verified_once, propagated) = maturity_histogram();
    assert_eq!(
        design + verified_once + propagated,
        REMEDIATION_CATALOG.len(),
        "histogram must partition the catalog"
    );
    assert_eq!(
        (design, verified_once, propagated),
        (1, 0, 0),
        "remediation-maturity ledger drifted — promote deliberately on a real re-scan pass, never round up"
    );

    let mut rounded: Vec<String> = Vec::new();
    for e in REMEDIATION_CATALOG {
        if e.maturity == RemediationMaturity::VerifiedOnce && e.verified_consumers.is_empty() {
            rounded.push(format!(
                "{:?}: claims VerifiedOnce with zero verified_consumers",
                e.finding
            ));
        }
        if e.maturity == RemediationMaturity::PropagatedFleetWide && e.verified_consumers.len() < 2
        {
            rounded.push(format!(
                "{:?}: claims PropagatedFleetWide with < 2 verified_consumers",
                e.finding
            ));
        }
    }
    assert!(
        rounded.is_empty(),
        "rounded-up remediation maturity claim(s):\n  - {}",
        rounded.join("\n  - ")
    );
}
