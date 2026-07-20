//! The security-REMEDIATION catalog (CATALOG REFLECTION) — a growable,
//! typed library of "which CVEs does our stack already know how to fix,
//! with which specific pin/overlay, applied to which consumers."
//!
//! Where [`crate::catalog`] answers "what gets checked" (the §4 verdict-
//! store schema) and [`crate::concern`] answers "what security concerns the
//! pipeline addresses" (the layered enjulho ship), this module answers a
//! third, distinct question the other two do not: **a CVE was fixed once —
//! where does that knowledge live so every OTHER consumer sharing the same
//! vulnerable layer can find and reuse it, instead of re-discovering and
//! re-fixing it independently?**
//!
//! Sibling schema, same crate (per `theory/SECURITY-LAYER.md` §10: "augment,
//! don't fork" — this is `gen-secattest`'s own territory, not a new
//! typescape). The remediation catalog is the *positive* counterpart to an
//! `image-scan` ignore-file: not "this finding is accepted risk," but "this
//! finding has a known, verified fix — apply it."
//!
//! **The two axes** (mirroring [`crate::catalog`] / [`crate::concern`]'s own
//! two-axis pattern, not inventing a third shape):
//!
//! - **The DATA axis** — [`RemediationEntry`]: the CVE id, the affected
//!   `(package, version-range)`, the [`Fix`] (a typed pin/overlay/patch
//!   reference — the mechanism, never prose), the [`AppliesTo`] dependency
//!   edge (which shared Nix layer/overlay/flake-input this remediation
//!   lives in), `verified_consumers` (repos that have ACTUALLY confirmed,
//!   via a real trivy re-scan, that the fix closes the CVE — never an
//!   assumption), and a [`RemediationMaturity`] (design → verified-once →
//!   propagated-fleet-wide).
//! - **The DELIVERY axis** — the closed [`CveFinding`] enum: every CVE a
//!   build-time gate (`theory/SECURITY-LAYER.md` §9's `image-scan`) has
//!   EVER hard-failed on, landed as ONE variant per finding, in the SAME
//!   commit as its [`RemediationEntry`] (CLOSED-LOOP MASS-SYNTHESIS rule
//!   1). Unlike [`crate::verdict::VerdictKind`] / [`crate::concern::Concern`]
//!   (closed by the domain itself), `CveFinding` is closed BY CURATION — a
//!   variant is added only when a real gate failure occurred, never
//!   speculatively for the open CVE universe. `CveFinding::cve_id` is an
//!   EXHAUSTIVE match with no wildcard arm: a new variant with no arm here
//!   is `error[E0004]: non-exhaustive patterns` at COMPILE time — the same
//!   forcing shape [`crate::verdict::VerdictKind::predicate_type`] uses.
//!   `tests/remediation_matrix.rs`'s `matrix_covers_every_known_finding`
//!   then closes the remaining gap a Rust match cannot: that every
//!   `CveFinding` variant ALSO has a catalog entry (a runtime CI
//!   forcing-function, mirroring `matrix_covers_every_catalogued_kind` /
//!   `matrix_covers_every_concern`) — never ship a variant without its row.
//!
//! **Tier-honest, never rounded up.** `RemediationMaturity::Design` means
//! the fix is known and may even be landed in a consumer's git history, but
//! is NOT yet confirmed by a real trivy re-scan. `VerifiedOnce` requires
//! ≥1 named `verified_consumers` entry; `PropagatedFleetWide` requires ≥2
//! (propagated beyond the single consumer that first hit the CVE). "The pin
//! landed in git" is evidence a fix EXISTS, never by itself evidence it
//! WORKS — `no_entry_claims_more_verification_than_it_has_evidence_for`
//! enforces the floor on the maturity a given `verified_consumers` count may
//! claim.

use crate::verdict::VerdictKind;

/// The closed, CURATED set of CVE findings a build-time gate has EVER
/// hard-failed on — the DELIVERY axis's forcing key. A new variant lands
/// here ONLY when a real gate failure occurred (never the speculative open
/// CVE universe), in the SAME commit as its [`RemediationEntry`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CveFinding {
    /// CVE-2026-39822 — Go stdlib, hard-failed a build-time CVE gate shared
    /// by six `akeylesslabs/akeyless-nix-images` services.
    Cve202639822GoStdlib,
}

impl CveFinding {
    /// Every known finding — the partition [`REMEDIATION_CATALOG`] and
    /// `tests/remediation_matrix.rs`'s `MATRIX` must both equal.
    pub const ALL: [CveFinding; 1] = [CveFinding::Cve202639822GoStdlib];

    /// The CVE identifier string this finding names.
    ///
    /// **The compile-time forcing function.** This match has NO wildcard
    /// arm. Add a `CveFinding` variant without adding an arm here and `cargo
    /// build` fails with:
    ///
    /// ```text
    /// error[E0004]: non-exhaustive patterns: `CveFinding::SomeNewVariant`
    ///               not covered
    ///   --> crates/gen-secattest/src/remediation.rs
    ///    |
    ///    | match self {
    ///    | ^^^^ pattern `CveFinding::SomeNewVariant` not covered
    /// ```
    ///
    /// This is the SAME exhaustive-match forcing shape
    /// [`crate::verdict::VerdictKind::predicate_type`] already uses — a new
    /// domain value cannot ship without every exhaustive consumer being
    /// updated, mechanically, by the compiler.
    #[must_use]
    pub fn cve_id(self) -> &'static str {
        match self {
            CveFinding::Cve202639822GoStdlib => "CVE-2026-39822",
        }
    }
}

/// ShipMaturity for a remediation: design → verified-once →
/// propagated-fleet-wide. Distinct from [`crate::concern::ShipMaturity`]
/// (that one grades live ENFORCEMENT of a concern; this one grades
/// CONFIRMED-CLOSURE evidence for a specific fix) — same shape, different
/// value set, so it is its own type rather than a forced reuse.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RemediationMaturity {
    /// The fix is named (a pin/overlay/patch) but NOT yet confirmed by a
    /// real trivy re-scan against any consumer. May already be landed in a
    /// consumer's git history — landed is not verified.
    Design,
    /// ≥1 real consumer has been re-scanned with trivy and the finding is
    /// confirmed gone (`verified_consumers` non-empty).
    VerifiedOnce,
    /// ≥2 independent consumers of the SAME shared layer have confirmed
    /// closure — the fix has propagated fleet-wide, not just proven once.
    PropagatedFleetWide,
}

impl RemediationMaturity {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            RemediationMaturity::Design => "design",
            RemediationMaturity::VerifiedOnce => "verified-once",
            RemediationMaturity::PropagatedFleetWide => "propagated-fleet-wide",
        }
    }

    /// The minimum `verified_consumers` count this maturity level requires
    /// as evidence — the floor `no_entry_claims_more_verification_than_it_
    /// has_evidence_for` checks every entry against.
    #[must_use]
    pub fn minimum_verified_consumers(self) -> usize {
        match self {
            RemediationMaturity::Design => 0,
            RemediationMaturity::VerifiedOnce => 1,
            RemediationMaturity::PropagatedFleetWide => 2,
        }
    }
}

/// The remediation MECHANISM itself — typed, never prose. Every entry names
/// HOW the CVE is closed, not just that a fix exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fix {
    /// Pin the affected package/toolchain to an exact fixed version.
    VersionPin { fixed_version: &'static str },
    /// Apply a Nix overlay that patches/replaces the vulnerable derivation.
    NixOverlay { overlay_name: &'static str },
    /// Apply a source patch to the vulnerable package.
    SourcePatch { patch_ref: &'static str },
    /// Re-pin a whole shared flake input past the upstream commit carrying
    /// the fix (used when no standalone package pin is expressible — e.g. a
    /// nixpkgs toolchain bump that isn't yet on the tracked release channel).
    FlakeInputRepin {
        input_name: &'static str,
        to_rev: &'static str,
    },
}

/// The dependency edge a remediation lives in — which shared Nix
/// layer/overlay/flake-input carries the fix, and which repo owns it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AppliesTo {
    /// The repo that owns the shared layer (`org/repo`).
    pub repo: &'static str,
    /// The specific shared Nix layer/overlay/flake-input name.
    pub layer: &'static str,
}

/// One typed point in the remediation catalog — a CVE's known fix.
#[derive(Clone, Debug)]
pub struct RemediationEntry {
    /// The closed finding key (the catalog's unique key / DELIVERY axis).
    pub finding: CveFinding,
    /// The CVE identifier — cross-checked against `finding.cve_id()`
    /// (`cve_ids_match_the_typed_finding`), never hand-duplicated silently.
    pub cve_id: &'static str,
    /// The affected package/toolchain.
    pub package: &'static str,
    /// The affected version range (free text — mirrors `engine`/`purpose`'s
    /// free-text style elsewhere in this crate; only the FIX mechanism is
    /// required to be typed, per `theory/SECURITY-LAYER.md` §10).
    pub version_range: &'static str,
    /// The fix mechanism (typed, not prose).
    pub fix: Fix,
    /// The dependency edge this remediation lives in.
    pub applies_to: AppliesTo,
    /// Repos/services that have ACTUALLY confirmed, via a real trivy
    /// re-scan, that the fix closes the CVE. Never populated on assumption.
    pub verified_consumers: &'static [&'static str],
    /// ShipMaturity: design → verified-once → propagated-fleet-wide.
    pub maturity: RemediationMaturity,
    /// One-line purpose / context.
    pub purpose: &'static str,
}

const fn entry(
    finding: CveFinding,
    cve_id: &'static str,
    package: &'static str,
    version_range: &'static str,
    fix: Fix,
    applies_to: AppliesTo,
    verified_consumers: &'static [&'static str],
    maturity: RemediationMaturity,
    purpose: &'static str,
) -> RemediationEntry {
    RemediationEntry {
        finding,
        cve_id,
        package,
        version_range,
        fix,
        applies_to,
        verified_consumers,
        maturity,
        purpose,
    }
}

/// CVE-2026-39822 — Go stdlib. Found live via real trivy scan data as ONE
/// shared root cause across 6 of the 9 `akeylesslabs/akeyless-nix-images`
/// services (`gator`, `bis`, `auth`, `logan`, `mark`, `kfm`), all built from
/// one shared Nix Go toolchain layer (`theory/SECURITY-LAYER.md` §10's
/// worked example).
///
/// **What IS confirmed (2026-07-20).** The fix landed live on
/// `akeylesslabs/akeyless-nix-images`'s `main` at commit
/// `7233190cdcca3d5e018345ee614bd9af5eb2974a` ("nixpkgs: bump to pick up
/// go_1_26 1.26.5 (CVE-2026-39822)") — verified directly by reading that
/// commit: `flake.nix`/`flake.lock` re-pin the `nixpkgs` input past
/// `nixos-25.11` to the first upstream rev (`4a07d79b…`) carrying
/// substitutable `go-1.26.5` for both `x86_64-linux` and `aarch64-linux`
/// (no local toolchain compile required). The repo's own `CLAUDE.md`, one
/// commit earlier (`acd8765`, 2026-07-19), independently corroborates the
/// PRE-fix state: `go_1_26` in the then-pinned nixpkgs resolved to 1.26.4,
/// "the newest packaged 1.26.x release... CVE-2026-39822 needs 1.26.5."
///
/// **What is NOT confirmed.** No real trivy re-scan evidence was found
/// proving the finding is actually gone post-bump — the repo's own CVE gate
/// (`zot-pull-scan` in `camelot-image-pipeline.yml`) had, as of the commit
/// immediately prior, never completed an end-to-end run (an ARC-runner
/// credential gap, per that workflow's own header, corroborated by
/// `CLAUDE.md`'s "No `.trivyignore` added... that pipeline has never
/// completed an end-to-end run"). Per the no-round-up rule: `maturity`
/// stays [`RemediationMaturity::Design`] and `verified_consumers` stays
/// EMPTY until a passing re-scan is actually observed — "the pin landed in
/// git" is evidence the fix EXISTS, never by itself evidence it WORKS.
const CVE_2026_39822_GO_STDLIB: RemediationEntry = entry(
    CveFinding::Cve202639822GoStdlib,
    "CVE-2026-39822",
    "go (stdlib, packaged as go_1_26 in nixpkgs)",
    "<1.26.5 (confirmed vulnerable: 1.26.4 — the newest 1.26.x nixpkgs had packaged pre-fix)",
    Fix::VersionPin {
        fixed_version: "1.26.5",
    },
    AppliesTo {
        repo: "akeylesslabs/akeyless-nix-images",
        layer: "shared Go toolchain Nix layer (go_1_26, resolved via the nixpkgs flake input)",
    },
    &[], // NOT verified by a real trivy re-scan yet — see doc comment above.
    RemediationMaturity::Design,
    "Go stdlib CVE shared across 6/9 akeyless-nix-images services (gator, bis, auth, logan, mark, kfm) — fix landed live @7233190, re-scan confirmation pending",
);

/// The full remediation catalog. Adding a fix = a `const` + one entry here +
/// one `CveFinding` variant + one matrix row in `tests/remediation_matrix.rs`
/// (same commit; CLOSED-LOOP MASS-SYNTHESIS rule 1 — never ship the catalog
/// with a gap, and never backfill a variant's row later).
pub const REMEDIATION_CATALOG: &[&RemediationEntry] = &[&CVE_2026_39822_GO_STDLIB];

/// Look up a finding's remediation entry.
#[must_use]
pub fn remediation_for(finding: CveFinding) -> Option<&'static RemediationEntry> {
    REMEDIATION_CATALOG.iter().copied().find(|e| e.finding == finding)
}

/// Look up a remediation entry by its raw CVE id string (the consumer-facing
/// lookup — a build-time gate hard-fails with a CVE id string, not a
/// `CveFinding` value).
#[must_use]
pub fn remediation_for_cve_id(cve_id: &str) -> Option<&'static RemediationEntry> {
    REMEDIATION_CATALOG.iter().copied().find(|e| e.cve_id == cve_id)
}

/// The confirmation-maturity histogram — `(design, verified_once,
/// propagated_fleet_wide)`. Sums to `REMEDIATION_CATALOG.len()`
/// (partition-complete; the test asserts it). This line IS the honest
/// ledger — promote an entry's maturity DELIBERATELY when a real trivy
/// re-scan actually confirms it, never round up.
#[must_use]
pub fn maturity_histogram() -> (usize, usize, usize) {
    let mut h = (0, 0, 0);
    for e in REMEDIATION_CATALOG {
        match e.maturity {
            RemediationMaturity::Design => h.0 += 1,
            RemediationMaturity::VerifiedOnce => h.1 += 1,
            RemediationMaturity::PropagatedFleetWide => h.2 += 1,
        }
    }
    h
}

/// A remediation entry's `finding` names the same [`VerdictKind::Cve`] slot
/// the six-clause law already gates on (S5 freshness / the CVE⊖VEX budget,
/// `crate::invariant::verify`) — named here so a reader can see the
/// remediation catalog and the verdict-store CATALOG are talking about the
/// SAME `Cve` verdict kind, not a parallel concept.
#[must_use]
pub const fn gated_verdict_kind() -> VerdictKind {
    VerdictKind::Cve
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_known_finding_has_exactly_one_catalog_entry() {
        // CATALOG REFLECTION: the catalog ⇄ the CveFinding partition. A new
        // finding cannot ship without an entry, and no entry names a
        // phantom finding.
        for finding in CveFinding::ALL {
            let n = REMEDIATION_CATALOG.iter().filter(|e| e.finding == finding).count();
            assert_eq!(n, 1, "finding {finding:?} must have exactly one catalog entry (found {n})");
        }
        assert_eq!(
            REMEDIATION_CATALOG.len(),
            CveFinding::ALL.len(),
            "catalog has extra/duplicate entries"
        );
    }

    #[test]
    fn cve_ids_match_the_typed_finding() {
        // The catalog's cve_id string must equal the typed finding's own
        // cve_id() — one source of truth, two witnesses (mirrors
        // catalog.rs's predicate_types_match_the_typed_kind).
        for e in REMEDIATION_CATALOG {
            assert_eq!(
                e.cve_id,
                e.finding.cve_id(),
                "cve_id drift for {:?}: entry says {:?}, finding says {:?}",
                e.finding,
                e.cve_id,
                e.finding.cve_id()
            );
        }
    }

    #[test]
    fn no_entry_claims_more_verification_than_it_has_evidence_for() {
        // The honesty gate: a maturity level cannot claim more verification
        // than its verified_consumers count actually earns. Design has no
        // floor; VerifiedOnce needs >=1; PropagatedFleetWide needs >=2.
        // (Never rounding up; rounding DOWN — fewer verified consumers
        // listed than the maturity's floor strictly requires — is the only
        // failure mode this test catches, by construction.)
        let mut failures: Vec<String> = Vec::new();
        for e in REMEDIATION_CATALOG {
            let floor = e.maturity.minimum_verified_consumers();
            if e.verified_consumers.len() < floor {
                failures.push(format!(
                    "{:?}: maturity {:?} requires >= {} verified_consumers, has {}",
                    e.finding,
                    e.maturity,
                    floor,
                    e.verified_consumers.len()
                ));
            }
        }
        assert!(failures.is_empty(), "rounded-up maturity claim(s):\n  - {}", failures.join("\n  - "));
    }

    #[test]
    fn maturity_histogram_is_the_honest_ledger() {
        // Tier-honest snapshot (2026-07-20): the one seed entry's fix is
        // known+landed but NOT yet trivy-re-scan-confirmed — Design. This
        // line IS the ledger; promote deliberately on a real re-scan pass.
        let (design, verified_once, propagated) = maturity_histogram();
        assert_eq!(
            design + verified_once + propagated,
            REMEDIATION_CATALOG.len(),
            "histogram must partition the catalog"
        );
        assert_eq!(
            (design, verified_once, propagated),
            (1, 0, 0),
            "remediation-maturity ledger drifted — update deliberately, never round up"
        );
    }

    #[test]
    fn the_seed_entry_names_cve_2026_39822_go_stdlib_correctly() {
        // Round-trip proxy (this crate's catalogs are plain typed consts,
        // not serde wire types — mirrors how concern.rs's ConcernSlot is
        // tested by direct field assertion, not JSON serialization).
        let e = remediation_for(CveFinding::Cve202639822GoStdlib).expect("seed entry must exist");
        assert_eq!(e.cve_id, "CVE-2026-39822");
        assert_eq!(e.package, "go (stdlib, packaged as go_1_26 in nixpkgs)");
        assert_eq!(
            e.fix,
            Fix::VersionPin {
                fixed_version: "1.26.5"
            }
        );
        assert_eq!(e.applies_to.repo, "akeylesslabs/akeyless-nix-images");
        assert!(e.applies_to.layer.contains("Go toolchain"));
        assert!(
            e.verified_consumers.is_empty(),
            "no real trivy re-scan evidence was found — verified_consumers must stay empty"
        );
        assert_eq!(
            e.maturity,
            RemediationMaturity::Design,
            "landed-but-unverified must not round up past Design"
        );

        // The cve_id-string lookup path (what a build-time gate hard-fails
        // with) resolves to the SAME entry.
        assert_eq!(remediation_for_cve_id("CVE-2026-39822").unwrap().finding, e.finding);
        assert!(remediation_for_cve_id("CVE-9999-00000").is_none());
    }

    #[test]
    fn the_remediation_catalog_names_the_same_cve_verdict_kind_the_law_gates_on() {
        assert_eq!(gated_verdict_kind(), VerdictKind::Cve);
    }
}
