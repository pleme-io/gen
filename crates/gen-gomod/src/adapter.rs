//! `GomodAdapter` — gen-gomod's implementation of the canonical
//! `gen_types::Adapter` trait.
//!
//! `build` + `confirm` are live (M1): they drive the per-package
//! incremental encoder ([`crate::interp::apply`]) over `go list -deps
//! -json` against a vendored tree. `lock`/`plan`/`diff`/`sbom` remain
//! typed-`Unsupported` until their milestones land — never a silent
//! wrong answer.

use std::path::PathBuf;

use gen_types::{
    Adapter, AdapterCtx, AdapterError, AdapterResult, ConfirmReport, DiffRef, DiffReport,
    InvariantBreak, LockOutcome, Plan, PlanIntent, Sbom, SbomFormat,
};

use crate::build_spec::{self, TargetTuple};
use crate::interp::{self, EncodeCtx, RealGoBuildEnv};

pub struct GomodAdapter;

impl GomodAdapter {
    /// Resolve the target tuple: an explicit `AdapterCtx.target` Rust
    /// triple (mapped to `goos/goarch`) or the build host.
    fn tuple_for(ctx: &AdapterCtx) -> TargetTuple {
        ctx.target
            .as_deref()
            .and_then(TargetTuple::from_rust_triple)
            .unwrap_or_else(TargetTuple::host)
    }

    /// Shared encode path for `build` + `confirm`.
    fn encode(ctx: &AdapterCtx) -> AdapterResult<build_spec::BuildSpec> {
        let manifest = ctx.workspace_root.join("go.mod");
        if !manifest.exists() {
            return Err(AdapterError::ManifestNotFound(manifest));
        }
        let ectx = EncodeCtx {
            root: ctx.workspace_root.clone(),
            tuple: Self::tuple_for(ctx),
        };
        let env = RealGoBuildEnv::default();
        interp::apply(&env, &ectx)
            .map_err(|e| AdapterError::Internal(format!("gomod encode: {e}")))
    }
}

impl Adapter for GomodAdapter {
    fn name(&self) -> &'static str {
        "gomod"
    }
    fn manifest_files(&self) -> &'static [&'static str] {
        &["go.mod"]
    }

    fn lock(&self, _ctx: &AdapterCtx) -> AdapterResult<LockOutcome> {
        // `go mod vendor` / `go mod tidy` — the resolver invocation
        // (network) — lands with the lock verb in a later milestone.
        Err(AdapterError::Unsupported("gomod lock not implemented yet".into()))
    }

    fn build(&self, ctx: &AdapterCtx) -> AdapterResult<gen_types::AdapterBuildSpec> {
        let spec = Self::encode(ctx)?;
        let data = serde_json::to_value(&spec)
            .map_err(|e| AdapterError::Internal(format!("serialize gomod build-spec: {e}")))?;
        Ok(gen_types::AdapterBuildSpec {
            ecosystem: "gomod".to_string(),
            schema_version: build_spec::SCHEMA_VERSION,
            data,
        })
    }

    fn plan(&self, _ctx: &AdapterCtx, _intent: &PlanIntent) -> AdapterResult<Plan> {
        Err(AdapterError::Unsupported("gomod plan not implemented yet".into()))
    }

    fn confirm(&self, ctx: &AdapterCtx) -> AdapterResult<ConfirmReport> {
        let spec = Self::encode(ctx)?;
        let mut held: Vec<String> = Vec::new();
        let mut broken: Vec<InvariantBreak> = Vec::new();

        let violations = crate::invariants::check(&spec);
        if violations.is_empty() {
            held.push("invariants::check".to_string());
        } else {
            for v in &violations {
                let (name, locus) = crate::invariants::violation_locus(v);
                let message = serde_json::to_string(v).unwrap_or_else(|_| format!("{v:?}"));
                broken.push(InvariantBreak { name: name.to_string(), message, locus });
            }
        }

        Ok(ConfirmReport { invariants_held: held, invariants_broken: broken })
    }

    fn diff(&self, _ctx: &AdapterCtx, _against: &DiffRef) -> AdapterResult<DiffReport> {
        Err(AdapterError::Unsupported("gomod diff not implemented yet".into()))
    }

    fn sbom(&self, _ctx: &AdapterCtx, _format: SbomFormat) -> AdapterResult<Sbom> {
        Err(AdapterError::Unsupported("gomod sbom not implemented yet".into()))
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

// Distributed-slice registration. gen-cli discovers this adapter at link
// time via the inventory iter — no per-adapter edit to gen-cli.
inventory::submit! {
    gen_types::AdapterRegistration {
        make: || Box::new(GomodAdapter),
        name: "gomod",
    }
}
