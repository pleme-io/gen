//! `AnsibleAdapter` — gen-ansible's implementation of the canonical
//! `gen_types::Adapter` trait. Stub-level today; every verb
//! returns `Unsupported` until the ansible-side parser lands.

use std::path::PathBuf;

use gen_types::{
    Adapter, AdapterCtx, AdapterError, AdapterResult, ConfirmReport, DiffRef, DiffReport,
    LockOutcome, Plan, PlanIntent, Sbom, SbomFormat,
};

pub struct AnsibleAdapter;

impl Adapter for AnsibleAdapter {
    fn name(&self) -> &'static str { "ansible" }
    fn manifest_files(&self) -> &'static [&'static str] { &["galaxy.yml"] }

    fn lock(&self, _ctx: &AdapterCtx) -> AdapterResult<LockOutcome> {
        Err(AdapterError::Unsupported("ansible lock not implemented".into()))
    }

    fn build(&self, _ctx: &AdapterCtx) -> AdapterResult<gen_types::AdapterBuildSpec> {
        Err(AdapterError::Unsupported("ansible build not implemented".into()))
    }

    fn plan(&self, _ctx: &AdapterCtx, _intent: &PlanIntent) -> AdapterResult<Plan> {
        Err(AdapterError::Unsupported("ansible plan not implemented".into()))
    }

    fn confirm(&self, _ctx: &AdapterCtx) -> AdapterResult<ConfirmReport> {
        Err(AdapterError::Unsupported("ansible confirm not implemented".into()))
    }

    fn diff(&self, _ctx: &AdapterCtx, _against: &DiffRef) -> AdapterResult<DiffReport> {
        Err(AdapterError::Unsupported("ansible diff not implemented".into()))
    }

    fn sbom(&self, _ctx: &AdapterCtx, _format: SbomFormat) -> AdapterResult<Sbom> {
        Err(AdapterError::Unsupported("ansible sbom not implemented".into()))
    }

    fn quirks_registry(&self) -> Vec<gen_types::AdapterQuirkEntry> {
        use gen_types::QuirkRegistry;
        <crate::quirks::AnsibleQuirks as QuirkRegistry>::registry()
            .into_iter()
            .map(|(p, qs)| gen_types::AdapterQuirkEntry {
                package: p.to_string(),
                quirks: qs.into_iter().filter_map(|q| serde_json::to_value(&q).ok()).collect(),
            })
            .collect()
    }
}

pub fn ctx_for(workspace_root: PathBuf) -> AdapterCtx {
    AdapterCtx { workspace_root, target: None }
}

// Distributed-slice registration. gen-cli discovers this adapter
// at link time via the inventory iter — no per-adapter edit to
// gen-cli when this crate ships.
inventory::submit! {
    gen_types::AdapterRegistration {
        make: || Box::new(AnsibleAdapter),
        name: "ansible",
    }
}
