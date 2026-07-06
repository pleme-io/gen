//! The triplet interpreter (item 3) — `apply(env, ctx) -> BuildSpec`.
//!
//! Walks the encoder phases over `go list -deps -json` output:
//!
//! ```text
//! read-go-mod → go-list → parse-go-list → reject-cgo + reject-asm (Go-I12)
//!   → per-node: relative-path (Go-I3) → read-source + source_hash (Go-I8)
//!   → resolve-imports (Go-I1) → embed (Go-I9) → tree (Go-I2)
//!   → roots/members → compact target_resolves → go_sum tie (Go-I7)
//! ```
//!
//! Every side effect — the `go list` subprocess AND source-file reads —
//! is abstracted behind the [`GoBuildEnv`] trait, so the whole encoder
//! is driven by a [`MockGoBuildEnv`](../tests) in unit tests with ZERO
//! `go` / filesystem dependency. The trait IS the testability contract
//! (TYPED-SPEC + INTERPRETER TRIPLET). A phase that cannot complete
//! returns a typed [`GomodError::Interp`] naming the phase — never a
//! silent wrong answer, never a `panic!`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use indexmap::IndexMap;

use crate::build_spec::{
    self, BuildSpec, BuildTree, DepMode, EmbedSpec, GoCompactTargetResolves, GoPackageArgs,
    GoTargetEdges, GoTargetResolve, ModuleSpec, PackageKind, PackageModuleRef, PackageSource,
    PackageSpec, Renderer, TargetTuple, SCHEMA_VERSION,
};
use crate::error::GomodError;
use crate::golist::parse_stream;
use crate::gomod::GoMod;

/// The Environment seam — the ONLY place the encoder touches the world.
/// Real builds shell `go list` + read files; tests mock both.
pub trait GoBuildEnv {
    /// Run `go list -deps -json` for the tuple over the module at `root`,
    /// returning the concatenated-JSON-object stream. MUST be hermetic
    /// (`-mod=vendor`, `GOPROXY=off`) on the real impl.
    fn go_list(&self, root: &Path, tuple: &TargetTuple) -> Result<String, GomodError>;

    /// Read a file's bytes — go.mod, go.sum, and each source/embed file
    /// (for `source_hash`). Errors are surfaced typed, never swallowed.
    fn read_file(&self, path: &Path) -> Result<Vec<u8>, GomodError>;
}

/// Production [`GoBuildEnv`]: shells `go` and reads the real filesystem.
/// `mod_mode` defaults to `"vendor"` — the M1 hermetic contract
/// (vendored tree, no proxy). The one legitimate non-vendor use is a
/// dep-free module (nothing to vendor), which the integration test
/// selects explicitly.
pub struct RealGoBuildEnv {
    pub mod_mode: String,
}

impl Default for RealGoBuildEnv {
    fn default() -> Self {
        Self { mod_mode: "vendor".to_string() }
    }
}

impl GoBuildEnv for RealGoBuildEnv {
    fn go_list(&self, root: &Path, tuple: &TargetTuple) -> Result<String, GomodError> {
        let mut cmd = Command::new("go");
        cmd.arg("list").arg("-deps").arg("-json");
        if !tuple.tags.is_empty() {
            cmd.arg("-tags").arg(tuple.tags.join(","));
        }
        cmd.arg("./...");
        cmd.current_dir(root)
            .env("GOOS", &tuple.goos)
            .env("GOARCH", &tuple.goarch)
            .env("CGO_ENABLED", "0")
            .env("GOFLAGS", format!("-mod={}", self.mod_mode))
            .env("GOPROXY", "off")
            // No interactive prompts, ever.
            .env("GIT_TERMINAL_PROMPT", "0");
        let out = cmd
            .output()
            .map_err(|e| GomodError::GoList(format!("spawn `go list`: {e}")))?;
        if !out.status.success() {
            return Err(GomodError::GoList(format!(
                "`go list` exited {}: {}",
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    fn read_file(&self, path: &Path) -> Result<Vec<u8>, GomodError> {
        std::fs::read(path).map_err(|source| GomodError::Io { path: path.to_path_buf(), source })
    }
}

/// Encoder inputs: the module root + the single target tuple M1 builds.
#[derive(Clone, Debug)]
pub struct EncodeCtx {
    pub root: PathBuf,
    pub tuple: TargetTuple,
}

/// The encoder. Produces a v2 `Incremental` [`BuildSpec`] — the
/// per-package build graph — from `go list` output + go.mod, hashing
/// every node's sources for the incremental cache key.
pub fn apply(env: &dyn GoBuildEnv, ctx: &EncodeCtx) -> Result<BuildSpec, GomodError> {
    // ── read-go-mod ──────────────────────────────────────────────────
    let gomod_path = ctx.root.join("go.mod");
    let gomod_bytes = env
        .read_file(&gomod_path)
        .map_err(|_| GomodError::ManifestNotFound(gomod_path.clone()))?;
    let gomod_text = String::from_utf8_lossy(&gomod_bytes);
    let module: ModuleSpec = GoMod::parse(&gomod_text).to_module_spec(DepMode::Vendored, None);

    // ── go-list → parse-go-list ──────────────────────────────────────
    let json = env.go_list(&ctx.root, &ctx.tuple)?;
    let listed = parse_stream(&json)?;

    // ── reject-cgo / reject-asm (Go-I12: fail closed, never mis-build) ─
    for p in &listed {
        if !p.standard && !p.cgo_files.is_empty() {
            return Err(GomodError::Interp {
                phase: "reject-cgo",
                detail: format!(
                    "package `{}` has cgo sources ({} file(s)); cgo nodes are deferred to M-cgo — \
                     an M1 (incremental, vendored) build must be a cgo-free subgraph",
                    p.import_path,
                    p.cgo_files.len()
                ),
            });
        }
        if !p.standard && !p.s_files.is_empty() {
            return Err(GomodError::Interp {
                phase: "reject-asm",
                detail: format!(
                    "package `{}` has assembly sources; per-node asm is deferred to M-asm \
                     (std asm lives inside the opaque std-tree)",
                    p.import_path
                ),
            });
        }
    }

    // import-path → kind index, for resolving edges to node keys.
    let kind_of: HashMap<&str, PackageKind> =
        listed.iter().map(|p| (p.import_path.as_str(), p.kind())).collect();

    let mut packages: std::collections::BTreeMap<String, PackageSpec> =
        std::collections::BTreeMap::new();
    let mut resolve: std::collections::BTreeMap<String, GoTargetEdges> =
        std::collections::BTreeMap::new();
    let mut mains: Vec<String> = Vec::new();

    for p in &listed {
        let kind = p.kind();
        let key = build_spec::node_key(&p.import_path, kind, &ctx.tuple);

        // ── resolve-imports (Go-I1) ──────────────────────────────────
        // Map each direct import through ImportMap (vendor/replace
        // rewrite), then to the imported package's node key.
        let mut import_keys: Vec<String> = Vec::with_capacity(p.imports.len());
        for imp in &p.imports {
            let actual = p.import_map.get(imp).map_or(imp.as_str(), String::as_str);
            // A resolved import absent from the closure keeps its raw
            // path (no fabricated key) so invariants::Go-I1 flags it
            // rather than the encoder silently inventing a node.
            let ik = kind_of.get(actual).copied().unwrap_or(PackageKind::Module);
            import_keys.push(build_spec::node_key(actual, ik, &ctx.tuple));
        }
        import_keys.sort();
        import_keys.dedup();

        // ── relative-path (Go-I3) + read-source + source_hash (Go-I8) ─
        let (source, source_hash) = if kind.is_std() {
            // Std nodes are content-addressed by the shared std-tree
            // derivation, not per-file — no source, no per-node hash.
            (PackageSource::Std, String::new())
        } else {
            let rel = relative_under(&p.dir, &p.root)
                .or_else(|| relative_under(&p.dir, &ctx.root.to_string_lossy()))
                .ok_or_else(|| GomodError::Interp {
                    phase: "relative-path",
                    detail: format!(
                        "package dir `{}` is not under module root `{}`",
                        p.dir, p.root
                    ),
                })?;
            let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
            for f in p.go_files.iter().chain(p.embed_files.iter()) {
                let abs = Path::new(&p.dir).join(f);
                let bytes = env.read_file(&abs).map_err(|e| GomodError::Interp {
                    phase: "read-source",
                    detail: format!("{}: {e}", abs.display()),
                })?;
                entries.push((f.clone(), bytes));
            }
            (
                PackageSource::Vendored { relative_path: rel },
                build_spec::source_hash(&entries),
            )
        };

        let mut env_map: IndexMap<String, String> = IndexMap::new();
        env_map.insert("GOOS".to_string(), ctx.tuple.goos.clone());
        env_map.insert("GOARCH".to_string(), ctx.tuple.goarch.clone());
        env_map.insert("CGO_ENABLED".to_string(), "0".to_string());

        let spec = PackageSpec {
            import_path: p.import_path.clone(),
            kind,
            source,
            tree: tree_for(kind),
            go_files: p.go_files.clone(),
            build_tags: ctx.tuple.tags.clone(),
            embed: EmbedSpec {
                patterns: p.embed_patterns.clone(),
                files: p.embed_files.clone(),
            },
            imports: import_keys.clone(),
            import_map: p.import_map.clone(),
            module: p.module.as_ref().map(|m| PackageModuleRef {
                path: m.path.clone(),
                version: m.version.clone(),
            }),
            source_hash,
            args: GoPackageArgs { gcflags: Vec::new(), ldflags: Vec::new(), env: env_map },
            quirks: Vec::new(),
        };

        resolve.insert(
            key.clone(),
            GoTargetEdges { imports: import_keys, import_map: p.import_map.clone() },
        );
        if matches!(kind, PackageKind::Main) && !p.dep_only {
            mains.push(key.clone());
        }
        packages.insert(key, spec);
    }

    // ── roots/members — deterministic (sorted); root = smallest main,
    //    else the first node (a lib-only module has no main). ──────────
    mains.sort();
    mains.dedup();
    let root_package = mains
        .first()
        .cloned()
        .or_else(|| packages.keys().next().cloned())
        .unwrap_or_default();
    let workspace_members = mains;

    // ── compact target_resolves (single tuple; M-multitarget-ready) ───
    let mut full: IndexMap<String, GoTargetResolve> = IndexMap::new();
    full.insert(ctx.tuple.suffix(), GoTargetResolve { packages: resolve });
    let target_resolves = Some(GoCompactTargetResolves::from_full(full));

    // ── go_sum freshness tie (Go-I7). Absent go.sum ⇒ empty content ⇒
    //    the canonical empty-string SHA-256 the delta gate matches. ────
    let go_sum_bytes = env.read_file(&ctx.root.join("go.sum")).unwrap_or_default();
    let go_sum_sha256 = Some(build_spec::go_sum_sha256(&go_sum_bytes));

    Ok(BuildSpec {
        version: SCHEMA_VERSION,
        renderer: Renderer::Incremental,
        module,
        packages,
        root_package,
        workspace_members,
        target_resolves,
        go_sum_sha256,
    })
}

/// Build-tree placement (Go-I2). M1 has only `Std`/`Module`/`Main`, all
/// `Target`. The seam: when `Cgo`/`Tool` kinds land (M-cgo), they return
/// `Host` — a classifier-arm addition, not a rewrite.
fn tree_for(kind: PackageKind) -> BuildTree {
    match kind {
        PackageKind::Std | PackageKind::Module | PackageKind::Main => BuildTree::Target,
    }
}

/// `dir` relative to `root` (both absolute). `dir == root` ⇒ `"."`.
/// `None` when `dir` is not under `root`.
fn relative_under(dir: &str, root: &str) -> Option<String> {
    let root = root.trim_end_matches('/');
    if dir == root {
        return Some(".".to_string());
    }
    dir.strip_prefix(&format!("{root}/")).map(str::to_string)
}
