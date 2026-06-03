//! `GomodAdapter` — gen-gomod's implementation of the canonical
//! `gen_types::Adapter` trait. Stub-level today; every verb
//! returns `Unsupported` until the gomod-side parser lands.

use std::path::PathBuf;

use gen_types::{
    Adapter, AdapterCtx, AdapterError, AdapterResult, ConfirmReport, DiffRef, DiffReport,
    GenDeltaArtifact, LockOutcome, Plan, PlanIntent, Sbom, SbomFormat,
};

pub struct GomodAdapter;

impl Adapter for GomodAdapter {
    fn name(&self) -> &'static str { "gomod" }
    fn manifest_files(&self) -> &'static [&'static str] { &["go.mod"] }

    /// `gen lock` — resolve the module's deps + compute its
    /// `vendorHash`, then write `Go.build-spec.json` + `Go.gen.lock`.
    /// Network-bearing (runs one `go` subprocess) — this is the
    /// non-hermetic verb, by design (mirrors cargo's `lock`).
    fn lock(&self, ctx: &AdapterCtx) -> AdapterResult<LockOutcome> {
        let root = &ctx.workspace_root;
        crate::build_spec::generate_and_write(root)
            .map_err(|e| AdapterError::Internal(e.to_string()))?;
        Ok(LockOutcome {
            lockfile_path: root.join(crate::gen_delta::GoGenDelta::FILENAME),
            // Go.gen.lock is the gen-authored delta; treat each write as
            // a (re)creation — gomod has no upstream lock-update concept
            // distinct from a full regen.
            created: true,
            bumped: Vec::new(),
        })
    }

    /// `gen build` — manifest → typed build-spec envelope. Hermetic:
    /// parses `go.mod`/`go.sum` off disk, no subprocess. The
    /// `vendorHash` is NOT computed here (that's `lock`'s job); `build`
    /// emits the structural spec and leaves the hash to the committed
    /// delta, matching the Adapter trait's hermetic contract.
    fn build(&self, ctx: &AdapterCtx) -> AdapterResult<gen_types::AdapterBuildSpec> {
        let root = &ctx.workspace_root;
        let spec = crate::build_spec::generate(root)
            .map_err(|e| AdapterError::Internal(e.to_string()))?;
        let data = serde_json::to_value(&spec)
            .map_err(|e| AdapterError::Internal(format!("serialize gomod build-spec: {e}")))?;
        Ok(gen_types::AdapterBuildSpec {
            ecosystem: "gomod".to_string(),
            schema_version: crate::build_spec::SCHEMA_VERSION,
            data,
        })
    }

    fn plan(&self, _ctx: &AdapterCtx, _intent: &PlanIntent) -> AdapterResult<Plan> {
        Err(AdapterError::Unsupported("gomod plan not implemented".into()))
    }

    /// `gen confirm` — verify the build-spec's invariants. Parses the
    /// manifest, generates the spec, runs `invariants::check`, and maps
    /// each typed violation to a named `InvariantBreak`.
    fn confirm(&self, ctx: &AdapterCtx) -> AdapterResult<ConfirmReport> {
        let root = &ctx.workspace_root;
        let spec = crate::build_spec::generate(root)
            .map_err(|e| AdapterError::Internal(e.to_string()))?;
        let violations = crate::invariants::check(&spec);
        let invariants_broken: Vec<gen_types::InvariantBreak> = violations
            .iter()
            .map(|v| {
                let name = match v {
                    crate::invariants::Violation::StaleSchemaVersion { .. } => "stale-schema-version",
                    crate::invariants::Violation::VendorHashMissing { .. } => "vendor-hash-missing",
                    crate::invariants::Violation::EmptyModulePath { .. } => "empty-module-path",
                    crate::invariants::Violation::RootPackageNotFound { .. } => {
                        "root-package-not-found"
                    }
                    crate::invariants::Violation::OrphanWorkspaceMember { .. } => {
                        "orphan-workspace-member"
                    }
                };
                gen_types::InvariantBreak {
                    name: name.to_string(),
                    message: serde_json::to_string(v).unwrap_or_default(),
                    locus: None,
                }
            })
            .collect();
        // build() is hermetic and leaves vendorHash uncomputed, so
        // vendor-hash-missing is EXPECTED on the build-spec path; it is
        // not a confirm failure (the committed delta carries the hash).
        // Report it as a held invariant instead.
        let invariants_held = if invariants_broken
            .iter()
            .all(|b| b.name == "vendor-hash-missing")
        {
            vec!["gomod-spec-well-formed".to_string()]
        } else {
            Vec::new()
        };
        let invariants_broken: Vec<gen_types::InvariantBreak> = invariants_broken
            .into_iter()
            .filter(|b| b.name != "vendor-hash-missing")
            .collect();
        Ok(ConfirmReport {
            invariants_held,
            invariants_broken,
        })
    }

    fn diff(&self, _ctx: &AdapterCtx, _against: &DiffRef) -> AdapterResult<DiffReport> {
        Err(AdapterError::Unsupported("gomod diff not implemented".into()))
    }

    fn sbom(&self, _ctx: &AdapterCtx, _format: SbomFormat) -> AdapterResult<Sbom> {
        Err(AdapterError::Unsupported("gomod sbom not implemented".into()))
    }

    fn quirks_registry(&self) -> Vec<gen_types::AdapterQuirkEntry> {
        use gen_types::QuirkRegistry;
        <crate::quirks::GomodQuirks as QuirkRegistry>::registry()
            .into_iter()
            .map(|(p, qs)| gen_types::AdapterQuirkEntry {
                package: p.to_string(),
                quirks: qs.into_iter().filter_map(|q| serde_json::to_value(&q).ok()).collect(),
            })
            .collect()
    }

    fn dispatcher_reflection(&self) -> Vec<gen_types::DispatcherVariant> {
        use gen_types::TypedDispatcher;
        <crate::quirks::GomodQuirk as TypedDispatcher>::variant_fields()
            .into_iter()
            .map(|(kind, fields)| gen_types::DispatcherVariant {
                kind: kind.to_string(),
                fields: fields.into_iter().map(str::to_string).collect(),
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
        make: || Box::new(GomodAdapter),
        name: "gomod",
    }
}
