//! The typed security-verdict border — slots, the sealed run capability,
//! and the freshness time-dimension.
//!
//! `/algorithmic-prowess-seal`: the best-fit construction for "an unverified
//! artifact has NO run path" is a **proof-carrying capability**
//! ([`RunToken`]) — a value whose only constructor is a passing
//! verification, so *holding one IS the proof* that the artifact's verdict
//! was complete, subject-bound, cryptographically-verified, and fresh. No
//! ML. Pure type-state + content-addressing — the top of the
//! smallest-sufficient ladder for the fail-closed invariant.
//!
//! The address type is reused, never forked: [`AttestationAddr`] IS
//! `gen_pdc::ContentAddr` — the THIRD use of PDC clause-4's one cell
//! (cache key = incremental boundary = attestation address).

use serde::{Deserialize, Serialize};

use gen_pdc::ContentAddr;

use crate::invariant::VerdictViolation;

/// The attestation address = the PDC content hash. THE THIRD USE of PDC
/// clause-4's one cell: `addr(n)` is simultaneously the cache key, the
/// incremental boundary, AND — here — the attestation address. Reused, not
/// forked, so an empty/malformed address is already unrepresentable
/// (it inherits `ContentAddr`'s parse-boundary seal — PDC clause 1 / S2).
pub type AttestationAddr = ContentAddr;

/// The verdict kinds the per-hash security verdict is composed of — the
/// world-class §4 verdict-store schema. Four are REQUIRED for completeness
/// (S1); the rest REFINE the required set (VEX subtracts from CVE; Scorecard
/// is a build-axis posture; VSA is the verifier's verdict-of-verdicts).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum VerdictKind {
    /// A1 — store-path / OCI signature (Nix ed25519 · cosign keyless).
    Signature,
    /// A4 — component inventory (sbomnix closure · Syft OCI fallback).
    Sbom,
    /// A5 — flaw detection (Trivy ∪ osv-scanner), keyed `(hash, cve_db_epoch)`.
    Cve,
    /// B1 — OpenVEX exploitability triage; the CVE gate reads `cve ⊖ vex`.
    Vex,
    /// A6 — build provenance (SLSA Provenance v1 / in-toto v1 / DSSE).
    Provenance,
    /// B4 — repo/build posture (OpenSSF Scorecard). Build axis, not per-hash.
    Scorecard,
    /// B3 — SLSA Verification Summary Attestation: the signed
    /// verdict-of-verdicts, the capstone a consumer/3PAO checks ONCE.
    Vsa,
}

impl VerdictKind {
    /// Every verdict kind the §4 store schema names.
    pub const ALL: [VerdictKind; 7] = [
        VerdictKind::Signature,
        VerdictKind::Sbom,
        VerdictKind::Cve,
        VerdictKind::Vex,
        VerdictKind::Provenance,
        VerdictKind::Scorecard,
        VerdictKind::Vsa,
    ];

    /// The four REQUIRED completeness slots (S1). A `CompleteVerdict` carries
    /// exactly these as non-optional fields — a missing one is unrepresentable.
    pub const REQUIRED: [VerdictKind; 4] = [
        VerdictKind::Signature,
        VerdictKind::Sbom,
        VerdictKind::Cve,
        VerdictKind::Provenance,
    ];

    /// Is this kind required for S1 completeness?
    #[must_use]
    pub fn required_for_completeness(self) -> bool {
        VerdictKind::REQUIRED.contains(&self)
    }

    /// TIMELESS (a pure function of the bytes → dedup-eligible, no TTL) vs
    /// TIME-BOUND (CVE / VEX — keyed to a `cve_db_epoch`, re-checked on TTL
    /// expiry even for unchanged bytes; the honest C2 external-world ceiling).
    #[must_use]
    pub fn is_timeless(self) -> bool {
        !matches!(self, VerdictKind::Cve | VerdictKind::Vex)
    }

    /// The standardized in-toto attestation-spec v1 `predicateType` URI this
    /// verdict blob is emitted under — so the store is GUAC-ingestible +
    /// 3PAO-recognizable by construction (stack doc §2 "confirm in-toto v1").
    #[must_use]
    pub fn predicate_type(self) -> &'static str {
        match self {
            // signature is a DSSE envelope, not an in-toto predicate itself.
            VerdictKind::Signature => "https://in-toto.io/attestation/link/v0.3",
            VerdictKind::Sbom => "https://cyclonedx.org/bom/v1.6",
            VerdictKind::Cve => "https://in-toto.io/attestation/vulns/v0.2",
            VerdictKind::Vex => "https://openvex.dev/ns/v0.2.0",
            VerdictKind::Provenance => "https://slsa.dev/provenance/v1",
            VerdictKind::Scorecard => "https://slsa.dev/scorecard/v0.1",
            VerdictKind::Vsa => "https://slsa.dev/verification_summary/v1",
        }
    }
}

/// The runtime verification outcome of a single attestation slot. The tool
/// (cosign/trivy/sbomnix/slsa-verifier) sets this; the CONTRACT is
/// fail-closed on it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SlotState {
    /// Cryptographically verified (signature checked, digest matched). The
    /// honest external-world (C2) boundary: that this flag reflects a REAL
    /// crypto pass rests on the tool's honesty + (for the hardest tier) a
    /// hardware attestation root this contract does not yet carry — a named
    /// gate, DESIGN, never claimed done.
    Verified,
    /// Verification RAN and FAILED (bad signature, digest mismatch, …).
    Failed(String),
    /// Never checked — the fail-closed DEFAULT. An `Unchecked` slot yields no
    /// run token (S3): absence of a verification is treated as failure.
    Unchecked,
}

impl SlotState {
    /// Only `Verified` passes. `Unchecked` and `Failed` both deny the run
    /// token — fail-closed (S3).
    #[must_use]
    pub fn is_verified(&self) -> bool {
        matches!(self, SlotState::Verified)
    }
}

/// A per-severity finding count. `saturating_sub` implements the CVE⊖VEX
/// subtraction; `within` is the gate comparison.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Severities {
    pub critical: u32,
    pub high: u32,
}

impl Severities {
    #[must_use]
    pub fn new(critical: u32, high: u32) -> Self {
        Severities { critical, high }
    }

    /// EXPLOITABLE = raw ⊖ vex-suppressed, per severity (saturating, never
    /// underflows). The world-class VEX move: the gate reads THIS, not raw.
    #[must_use]
    pub fn saturating_sub(self, other: Self) -> Self {
        Severities {
            critical: self.critical.saturating_sub(other.critical),
            high: self.high.saturating_sub(other.high),
        }
    }

    /// `self ≤ max` on every severity — the fail-closed CVE threshold gate.
    #[must_use]
    pub fn within(self, max: Self) -> bool {
        self.critical <= max.critical && self.high <= max.high
    }
}

/// The CVE-database epoch a scan ran against — the time dimension of the CVE
/// slot. Monotonic; a later epoch may disclose CVEs against bytes an earlier
/// scan cleared (why the CVE slot is time-bound and never timeless).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CveDbEpoch(pub u64);

/// Maximum epochs of staleness a CVE scan may carry before it must be
/// re-run (S5). A `ttl = 0` means "must be scanned against the current epoch."
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EpochTtl(pub u64);

/// The SBOM serialization format (CM-8 inventory). CycloneDX 1.6 is the
/// current world-class default; SPDX the alternative.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SbomFormat {
    /// CycloneDX at the given minor (world-class default: 6 → v1.6).
    CycloneDx { minor: u8 },
    /// SPDX at the given minor.
    Spdx { minor: u8 },
}

/// SLSA build level (v1). Declaration order is the total order used by the
/// provenance gate (`slsa_level >= min`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SlsaLevel {
    L0,
    L1,
    L2,
    L3,
}

// ── the four REQUIRED slots ─────────────────────────────────────────────

/// A1 — the signature attestation slot.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignatureSlot {
    /// The content hash this signature attests (S4 subject-bound).
    pub subject: AttestationAddr,
    /// The verification outcome (S3/S4).
    pub state: SlotState,
    /// The signing identity — a keyless Fulcio subject or a key fingerprint.
    pub identity: String,
}

/// A4 — the SBOM attestation slot.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SbomSlot {
    pub subject: AttestationAddr,
    pub state: SlotState,
    pub format: SbomFormat,
    /// Component count — a nonzero inventory. An empty SBOM is not a real
    /// CM-8 inventory and fails the gate (`SbomEmpty`).
    pub component_count: u32,
}

/// A5 — the CVE attestation slot, carrying the (hash, cve_db_epoch)+TTL
/// time dimension AND the VEX subtraction (raw ⊖ vex-suppressed).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CveSlot {
    pub subject: AttestationAddr,
    pub state: SlotState,
    /// The CVE-DB epoch this scan ran against (S5 time dimension).
    pub scanned_epoch: CveDbEpoch,
    /// Raw findings before VEX suppression.
    pub raw: Severities,
    /// VEX-suppressed (not-affected / not-reachable) findings — the OpenVEX
    /// leg. `exploitable = raw ⊖ vex_suppressed`.
    pub vex_suppressed: Severities,
}

impl CveSlot {
    /// EXPLOITABLE findings = raw ⊖ vex-suppressed. The fail-closed gate
    /// reads THIS (B1: the CVE gate is `cve ⊖ vex`, not raw noise).
    #[must_use]
    pub fn exploitable(&self) -> Severities {
        self.raw.saturating_sub(self.vex_suppressed)
    }

    /// Is this scan stale (S5)? True when it ran more than `ttl` epochs
    /// behind the current epoch — even for unchanged bytes (the C2 ceiling).
    #[must_use]
    pub fn is_stale(&self, ctx: &VerifyContext) -> bool {
        ctx.current_epoch.0.saturating_sub(self.scanned_epoch.0) > ctx.ttl.0
    }
}

/// A6 — the provenance attestation slot.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProvenanceSlot {
    pub subject: AttestationAddr,
    pub state: SlotState,
    /// The SLSA build level this provenance claims (SR-3/4).
    pub slsa_level: SlsaLevel,
}

// ── the incomplete (wire/builder) form + the complete (sealed) form ─────

/// The wire / builder form of a verdict — each required slot is OPTIONAL, so
/// a producer can assemble it incrementally and it can deserialize from a
/// partially-populated store. It is DELIBERATELY incompletable; the only way
/// to a run path is through [`PartialVerdict::into_complete`] (S1 parse-time
/// rejection) followed by [`crate::invariant::verify`] (S3 fail-closed).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PartialVerdict {
    /// The artifact's attestation address (S2).
    pub subject: AttestationAddr,
    #[serde(default)]
    pub signature: Option<SignatureSlot>,
    #[serde(default)]
    pub sbom: Option<SbomSlot>,
    #[serde(default)]
    pub cve: Option<CveSlot>,
    #[serde(default)]
    pub provenance: Option<ProvenanceSlot>,
}

impl PartialVerdict {
    /// A partial verdict for `subject` with no slots yet (the builder start).
    #[must_use]
    pub fn empty(subject: AttestationAddr) -> Self {
        PartialVerdict {
            subject,
            signature: None,
            sbom: None,
            cve: None,
            provenance: None,
        }
    }

    /// Which required kinds are absent (the completeness gap).
    #[must_use]
    pub fn missing(&self) -> Vec<VerdictKind> {
        let mut m = Vec::new();
        if self.signature.is_none() {
            m.push(VerdictKind::Signature);
        }
        if self.sbom.is_none() {
            m.push(VerdictKind::Sbom);
        }
        if self.cve.is_none() {
            m.push(VerdictKind::Cve);
        }
        if self.provenance.is_none() {
            m.push(VerdictKind::Provenance);
        }
        m
    }

    /// S1 — the completeness parse-boundary. A missing slot is REJECTED here;
    /// past this point a `CompleteVerdict` is a product with no `Option`, so
    /// incompleteness is truly-unrepresentable downstream.
    ///
    /// # Errors
    /// [`IncompleteVerdict`] naming every absent required slot.
    pub fn into_complete(self) -> Result<CompleteVerdict, IncompleteVerdict> {
        let missing = self.missing();
        if !missing.is_empty() {
            return Err(IncompleteVerdict {
                subject: self.subject,
                missing,
            });
        }
        Ok(CompleteVerdict {
            subject: self.subject,
            // unwraps are provably safe: `missing().is_empty()` ⇒ all Some.
            signature: self.signature.expect("completeness checked"),
            sbom: self.sbom.expect("completeness checked"),
            cve: self.cve.expect("completeness checked"),
            provenance: self.provenance.expect("completeness checked"),
        })
    }
}

/// S1's forbidden state, surfaced at the parse boundary — a verdict missing
/// one or more required slots. Convertible to the universal violation channel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IncompleteVerdict {
    pub subject: AttestationAddr,
    pub missing: Vec<VerdictKind>,
}

impl IncompleteVerdict {
    /// Fold into the universal violation channel — one `MissingSlot` per gap.
    #[must_use]
    pub fn into_violations(self) -> Vec<VerdictViolation> {
        self.missing
            .into_iter()
            .map(|kind| VerdictViolation::MissingSlot {
                subject: self.subject.clone(),
                kind,
            })
            .collect()
    }
}

impl std::fmt::Display for IncompleteVerdict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "incomplete security verdict for {}: missing {} required slot(s) {:?}",
            self.subject.as_str(),
            self.missing.len(),
            self.missing
        )
    }
}

impl std::error::Error for IncompleteVerdict {}

/// The COMPLETE, sealed verdict — a product of all four required slots with
/// NO `Option`. Its existence IS proof of S1 (completeness): there is no
/// expressible path to construct it with a slot absent. Verification
/// ([`crate::invariant::verify`]) turns it into a [`RunToken`] or a list of
/// violations — fail-closed (S3).
#[derive(Clone, Debug, Serialize)]
pub struct CompleteVerdict {
    pub subject: AttestationAddr,
    pub signature: SignatureSlot,
    pub sbom: SbomSlot,
    pub cve: CveSlot,
    pub provenance: ProvenanceSlot,
}

impl CompleteVerdict {
    /// The four required slots as `(kind, subject, state)` triples — the
    /// input the checker walks for S4 (subject-bound) + S3 (verified).
    #[must_use]
    pub fn required_slots(&self) -> [(VerdictKind, &AttestationAddr, &SlotState); 4] {
        [
            (VerdictKind::Signature, &self.signature.subject, &self.signature.state),
            (VerdictKind::Sbom, &self.sbom.subject, &self.sbom.state),
            (VerdictKind::Cve, &self.cve.subject, &self.cve.state),
            (VerdictKind::Provenance, &self.provenance.subject, &self.provenance.state),
        ]
    }
}

impl<'de> Deserialize<'de> for CompleteVerdict {
    /// S1 at the WIRE boundary — a JSON verdict missing a required slot is an
    /// `Err` here, never a value that flows downstream. Routes through the
    /// lenient [`PartialVerdict`] then [`PartialVerdict::into_complete`], so
    /// the parse-time rejection is exact (mirrors `ContentAddr::deserialize`).
    fn deserialize<D>(d: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let partial = PartialVerdict::deserialize(d)?;
        partial
            .into_complete()
            .map_err(serde::de::Error::custom)
    }
}

/// The external-world facts verification depends on — the C2 ceiling inputs.
/// The current CVE-DB epoch + TTL (S5 freshness), the max EXPLOITABLE CVE
/// budget (the CVE⊖VEX gate), and the minimum SLSA level.
#[derive(Clone, Copy, Debug)]
pub struct VerifyContext {
    pub current_epoch: CveDbEpoch,
    pub ttl: EpochTtl,
    /// Max EXPLOITABLE (post-VEX) findings tolerated — the fail-closed gate.
    pub cve_thresholds: Severities,
    /// Minimum acceptable SLSA build level (SR-3/4).
    pub min_slsa_level: SlsaLevel,
}

impl VerifyContext {
    /// The strict default posture: current epoch `now`, TTL 0 (must be
    /// scanned against the current epoch), ZERO exploitable criticals/highs,
    /// SLSA L3. The world-class enforce-not-audit setpoint.
    #[must_use]
    pub fn strict(now: CveDbEpoch) -> Self {
        VerifyContext {
            current_epoch: now,
            ttl: EpochTtl(0),
            cve_thresholds: Severities::new(0, 0),
            min_slsa_level: SlsaLevel::L3,
        }
    }
}

/// A proof-carrying RUN capability (S3, fail-closed). Its ONLY minter is
/// [`crate::invariant::verify`] via the crate-private [`RunToken::seal`] —
/// there is no public constructor, no public field, no `Default`. Holding a
/// `RunToken` IS the proof that the artifact at [`RunToken::addr`] carried a
/// COMPLETE, subject-bound, cryptographically-verified, fresh security
/// verdict. An unverified artifact has NO expressible path to one.
///
/// (The last mile — the kernel actually refusing to `execve` bytes without a
/// token — is the eBPF void the stack doc §Tier-C names: a named EXTERNAL
/// gate, DESIGN, never claimed done. Within the typed pipeline, "no run path
/// without a token" is truly-unrepresentable.)
#[derive(Clone, Debug)]
pub struct RunToken {
    addr: AttestationAddr,
    /// The epoch the CVE slot was fresh against when the token was minted —
    /// carried for audit (a token is only as fresh as this epoch).
    verified_at_epoch: CveDbEpoch,
    /// The required kinds that verified — the audit witness.
    verified_kinds: Vec<VerdictKind>,
}

impl RunToken {
    /// The crate-private seal — the SOLE minter, called only by a passing
    /// [`crate::invariant::verify`]. `pub(crate)` means there is no path to a
    /// token outside this crate (truly-unrep externally); inside, only the
    /// checker calls it (fail-closed).
    #[must_use]
    pub(crate) fn seal(
        addr: AttestationAddr,
        verified_at_epoch: CveDbEpoch,
        verified_kinds: Vec<VerdictKind>,
    ) -> Self {
        RunToken {
            addr,
            verified_at_epoch,
            verified_kinds,
        }
    }

    /// The proven attestation address — the artifact this token admits.
    #[must_use]
    pub fn addr(&self) -> &AttestationAddr {
        &self.addr
    }

    /// The epoch the CVE slot was fresh against at mint time.
    #[must_use]
    pub fn verified_at_epoch(&self) -> CveDbEpoch {
        self.verified_at_epoch
    }

    /// The required kinds that verified (the audit witness — always the four
    /// required slots for a well-formed mint).
    #[must_use]
    pub fn verified_kinds(&self) -> &[VerdictKind] {
        &self.verified_kinds
    }
}
