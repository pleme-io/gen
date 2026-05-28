//! `NpmAdapter` — gen-npm's implementation of the canonical
//! `gen_types::Adapter` trait.
//!
//! Stub-level v1: every verb is wired but most return `Unsupported`
//! until the npm-side spec emitter / planner / SBOM emitter land.
//! The point of shipping the stub now is the routing surface — the
//! moment NpmAdapter implements `build`, substrate's `npm.<shape>`
//! wrappers light up the same six flake apps + IFD machinery that
//! `rust.<shape>` already has.

use std::path::PathBuf;

use gen_types::{
    Adapter, AdapterCtx, AdapterError, AdapterResult, ConfirmReport, DiffRef, DiffReport,
    LockOutcome, Plan, PlanIntent, Sbom, SbomFormat,
};

pub struct NpmAdapter;

impl Adapter for NpmAdapter {
    fn name(&self) -> &'static str {
        "npm"
    }

    fn manifest_files(&self) -> &'static [&'static str] {
        &["package.json"]
    }

    fn lock(&self, _ctx: &AdapterCtx) -> AdapterResult<LockOutcome> {
        Err(AdapterError::Unsupported(
            "npm lock not implemented yet (M1)".to_string(),
        ))
    }

    fn build(&self, _ctx: &AdapterCtx) -> AdapterResult<gen_types::AdapterBuildSpec> {
        Err(AdapterError::Unsupported(
            "npm build not implemented yet (M1)".to_string(),
        ))
    }

    fn plan(&self, _ctx: &AdapterCtx, _intent: &PlanIntent) -> AdapterResult<Plan> {
        Err(AdapterError::Unsupported(
            "npm plan not implemented yet (M2)".to_string(),
        ))
    }

    fn confirm(&self, _ctx: &AdapterCtx) -> AdapterResult<ConfirmReport> {
        Err(AdapterError::Unsupported(
            "npm confirm not implemented yet (M1)".to_string(),
        ))
    }

    fn diff(&self, _ctx: &AdapterCtx, _against: &DiffRef) -> AdapterResult<DiffReport> {
        Err(AdapterError::Unsupported(
            "npm diff not implemented yet (M2)".to_string(),
        ))
    }

    fn sbom(&self, _ctx: &AdapterCtx, _format: SbomFormat) -> AdapterResult<Sbom> {
        Err(AdapterError::Unsupported(
            "npm sbom not implemented yet (M2)".to_string(),
        ))
    }

    /// Expose the typed NpmQuirk registry through the adapter
    /// envelope. Currently empty — entries land as we encounter
    /// real upstream npm packages needing class-helper workarounds.
    fn quirks_registry(&self) -> Vec<gen_types::AdapterQuirkEntry> {
        use gen_types::QuirkRegistry;
        <crate::quirks::NpmQuirks as QuirkRegistry>::registry()
            .into_iter()
            .map(|(p, qs)| gen_types::AdapterQuirkEntry {
                package: p.to_string(),
                quirks: qs
                    .into_iter()
                    .filter_map(|q| serde_json::to_value(&q).ok())
                    .collect(),
            })
            .collect()
    }

    fn dispatcher_reflection(&self) -> Vec<gen_types::DispatcherVariant> {
        use gen_types::TypedDispatcher;
        <crate::quirks::NpmQuirk as TypedDispatcher>::variant_fields()
            .into_iter()
            .map(|(kind, fields)| gen_types::DispatcherVariant {
                kind: kind.to_string(),
                fields: fields.into_iter().map(str::to_string).collect(),
            })
            .collect()
    }
}

#[must_use]
pub fn ctx_for(workspace_root: PathBuf) -> AdapterCtx {
    AdapterCtx {
        workspace_root,
        target: None,
    }
}

inventory::submit! {
    gen_types::AdapterRegistration {
        make: || Box::new(NpmAdapter),
        name: "npm",
    }
}
