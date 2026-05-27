//! `BundlerAdapter` — gen-bundler's implementation of the canonical
//! `gen_types::Adapter` trait.
//!
//! Stub-level v1: routing surface, all verbs return `Unsupported`.
//! Same pattern as `gen-npm` — landing the trait impl now means
//! substrate's `ruby.<shape>` wrappers light up the moment any verb
//! grows a real implementation.

use std::path::PathBuf;

use gen_types::{
    Adapter, AdapterCtx, AdapterError, AdapterResult, ConfirmReport, DiffRef, DiffReport,
    LockOutcome, Plan, PlanIntent, Sbom, SbomFormat,
};

pub struct BundlerAdapter;

impl Adapter for BundlerAdapter {
    fn name(&self) -> &'static str {
        "bundler"
    }

    fn manifest_files(&self) -> &'static [&'static str] {
        &["Gemfile"]
    }

    fn lock(&self, _ctx: &AdapterCtx) -> AdapterResult<LockOutcome> {
        Err(AdapterError::Unsupported(
            "bundler lock not implemented yet (M1)".to_string(),
        ))
    }

    fn build(&self, _ctx: &AdapterCtx) -> AdapterResult<gen_types::AdapterBuildSpec> {
        Err(AdapterError::Unsupported(
            "bundler build not implemented yet (M1)".to_string(),
        ))
    }

    fn plan(&self, _ctx: &AdapterCtx, _intent: &PlanIntent) -> AdapterResult<Plan> {
        Err(AdapterError::Unsupported(
            "bundler plan not implemented yet (M2)".to_string(),
        ))
    }

    fn confirm(&self, _ctx: &AdapterCtx) -> AdapterResult<ConfirmReport> {
        Err(AdapterError::Unsupported(
            "bundler confirm not implemented yet (M1)".to_string(),
        ))
    }

    fn diff(&self, _ctx: &AdapterCtx, _against: &DiffRef) -> AdapterResult<DiffReport> {
        Err(AdapterError::Unsupported(
            "bundler diff not implemented yet (M2)".to_string(),
        ))
    }

    fn sbom(&self, _ctx: &AdapterCtx, _format: SbomFormat) -> AdapterResult<Sbom> {
        Err(AdapterError::Unsupported(
            "bundler sbom not implemented yet (M2)".to_string(),
        ))
    }
}

#[must_use]
pub fn ctx_for(workspace_root: PathBuf) -> AdapterCtx {
    AdapterCtx {
        workspace_root,
        target: None,
    }
}
