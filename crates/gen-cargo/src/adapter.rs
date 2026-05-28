//! `CargoAdapter` — gen-cargo's implementation of the canonical
//! `gen_types::Adapter` trait.
//!
//! Cargo-shaped wrapper around the existing build_spec / invariants
//! / features pipeline. Every operator-facing CLI verb (`gen lock`,
//! `gen build`, `gen confirm`, …) routes through this trait so the
//! cross-ecosystem surface stays uniform.

use std::path::PathBuf;

use gen_types::{
    Adapter, AdapterCtx, AdapterError, AdapterResult, ConfirmReport, DiffRef, DiffReport,
    InvariantBreak, LockOutcome, Plan, PlanIntent, Sbom, SbomFormat,
};

use crate::build_spec;

/// Singleton routing handle. Adapters are stateless — the workspace
/// path comes from the per-call `AdapterCtx`.
pub struct CargoAdapter;

impl Adapter for CargoAdapter {
    fn name(&self) -> &'static str {
        "cargo"
    }

    fn manifest_files(&self) -> &'static [&'static str] {
        &["Cargo.toml"]
    }

    fn lock(&self, ctx: &AdapterCtx) -> AdapterResult<LockOutcome> {
        // v1: shell out to cargo. Hermetic implementations land in
        // a future revision when we drop the cargo-metadata
        // dependency for `build`. `lock` legitimately needs the
        // resolver (network access); it stays as the
        // intentionally-non-hermetic operator verb.
        use std::process::Command;
        let lockfile = ctx.workspace_root.join("Cargo.lock");
        let created = !lockfile.exists();
        let status = Command::new("cargo")
            .arg("generate-lockfile")
            .current_dir(&ctx.workspace_root)
            .status()
            .map_err(|e| AdapterError::Internal(format!("cargo generate-lockfile spawn: {e}")))?;
        if !status.success() {
            return Err(AdapterError::Internal(format!(
                "cargo generate-lockfile exited with status {status:?}"
            )));
        }
        Ok(LockOutcome {
            lockfile_path: lockfile,
            created,
            bumped: Vec::new(),
        })
    }

    fn build(&self, ctx: &AdapterCtx) -> AdapterResult<gen_types::AdapterBuildSpec> {
        let manifest = ctx.workspace_root.join("Cargo.toml");
        if !manifest.exists() {
            return Err(AdapterError::ManifestNotFound(manifest));
        }
        let lockfile = ctx.workspace_root.join("Cargo.lock");
        if !lockfile.exists() {
            return Err(AdapterError::LockfileNotFound(lockfile));
        }

        let spec = match &ctx.target {
            Some(t) => build_spec::generate_for_target(&ctx.workspace_root, t),
            None => build_spec::generate(&ctx.workspace_root),
        }
        .map_err(|e| AdapterError::Internal(format!("build_spec generate: {e}")))?;

        // Serialize through serde_json::Value so the cross-ecosystem
        // envelope carries the cargo-shaped payload opaquely.
        let data = serde_json::to_value(&spec)
            .map_err(|e| AdapterError::Internal(format!("serialize build-spec: {e}")))?;

        Ok(gen_types::AdapterBuildSpec {
            ecosystem: "cargo".to_string(),
            schema_version: build_spec::SCHEMA_VERSION,
            data,
        })
    }

    fn plan(&self, _ctx: &AdapterCtx, _intent: &PlanIntent) -> AdapterResult<Plan> {
        Err(AdapterError::Unsupported(
            "cargo plan is not implemented yet (M2)".to_string(),
        ))
    }

    fn confirm(&self, ctx: &AdapterCtx) -> AdapterResult<ConfirmReport> {
        // Run the existing invariants pass; convert outcomes to the
        // typed report shape.
        let spec = match &ctx.target {
            Some(t) => build_spec::generate_for_target(&ctx.workspace_root, t),
            None => build_spec::generate(&ctx.workspace_root),
        }
        .map_err(|e| AdapterError::Internal(format!("generate for confirm: {e}")))?;

        let mut held: Vec<String> = Vec::new();
        let mut broken: Vec<InvariantBreak> = Vec::new();

        let violations = crate::invariants::check(&spec);
        if violations.is_empty() {
            held.push("invariants::check".to_string());
        } else {
            for v in &violations {
                let (name, locus) = violation_locus(v);
                let message = serde_json::to_string(v)
                    .unwrap_or_else(|_| format!("{v:?}"));
                broken.push(InvariantBreak {
                    name: name.to_string(),
                    message,
                    locus,
                });
            }
        }

        Ok(ConfirmReport {
            invariants_held: held,
            invariants_broken: broken,
        })
    }

    fn diff(&self, _ctx: &AdapterCtx, _against: &DiffRef) -> AdapterResult<DiffReport> {
        Err(AdapterError::Unsupported(
            "cargo diff is not implemented yet (M2)".to_string(),
        ))
    }

    fn sbom(&self, _ctx: &AdapterCtx, _format: SbomFormat) -> AdapterResult<Sbom> {
        Err(AdapterError::Unsupported(
            "cargo sbom is not implemented yet (M2)".to_string(),
        ))
    }
}

/// Construct an `AdapterCtx` from a workspace root path. Convenience
/// for CLI bridges and tests.
#[must_use]
pub fn ctx_for(workspace_root: PathBuf) -> AdapterCtx {
    AdapterCtx {
        workspace_root,
        target: None,
    }
}

/// Convert a `Violation` variant into a stable rule name + optional
/// locus pointing at the offending key. Used by the `confirm` verb's
/// typed report.
fn violation_locus(v: &crate::invariants::Violation) -> (&'static str, Option<String>) {
    use crate::invariants::Violation::*;
    match v {
        UnresolvedDep { from, .. } => ("unresolved-dep", Some(from.clone())),
        RegistryWithoutSha256 { crate_key, .. } => {
            ("registry-without-sha256", Some(crate_key.clone()))
        }
        WorkspaceMemberNotInCrates { key } => {
            ("workspace-member-not-in-crates", Some(key.clone()))
        }
        RootCrateNotInCrates { key } => ("root-crate-not-in-crates", Some(key.clone())),
        DevDepInRuntimeOrBuild { from, .. } => {
            ("dev-dep-in-runtime-or-build", Some(from.clone()))
        }
        RenameVersionMismatch { from, .. } => ("rename-version-mismatch", Some(from.clone())),
        DuplicateCrateKey { key } => ("duplicate-crate-key", Some(key.clone())),
        MissingBuildRustCrateName { crate_key, .. } => {
            ("missing-build-rust-crate-name", Some(crate_key.clone()))
        }
        MissingUniversalPreBuild { crate_key, .. } => {
            ("missing-universal-pre-build", Some(crate_key.clone()))
        }
        RegistryUrlNotCanonical { crate_key, .. } => {
            ("registry-url-not-canonical", Some(crate_key.clone()))
        }
        StaleSchemaVersion { .. } => ("stale-schema-version", None),
        WorkspaceMemberMissingLibTarget { key, .. } => {
            ("workspace-member-missing-lib-target", Some(key.clone()))
        }
    }
}
