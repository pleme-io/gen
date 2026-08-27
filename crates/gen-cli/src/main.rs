//! `gen` — operator CLI for the universal package-manager engine.
//!
//! Subcommands:
//!   - `check <path>`       parse the workspace + print the typed manifest
//!   - `lock <path>`        report on the lockfile shape
//!   - `config-show <tier>` dump the typed config at a tier
//!   - `config-diff <a> <b>` compare two tiers
//!   - `adapters`           list the configured adapter routing
//!
//! Output format defaults to JSON; pass `--format yaml` for YAML.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use gen_config::GenConfig;
use shikumi::{ConfigTier, TieredConfig};

mod adopt_cli;
mod flake_lint;
mod fleet_migrate_plan;

// Force-link every adapter crate so its `inventory::submit!` runs
// at static-init time and gen-cli can discover the adapter via
// `gen_types::registered_adapters()`. Without these uses, Cargo's
// linker would drop the modules and the inventory iter would return
// an incomplete set. Each new adapter scaffolded via
// `gen scaffold-adapter` adds one line here.
#[allow(unused_imports)]
use {
    gen_ansible as _, gen_bundler as _, gen_cargo as _, gen_gomod as _, gen_helm as _,
    gen_npm as _, gen_pip as _, gen_poetry as _, gen_swift as _,
};

#[derive(Parser, Debug)]
#[command(
    name = "gen",
    version,
    about = "universal package-manager engine — operator CLI"
)]
struct Cli {
    /// Output format for structured commands.
    #[arg(long, value_enum, default_value_t = OutputFormat::Json, global = true)]
    format: OutputFormat,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum OutputFormat {
    Json,
    Yaml,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Parse the workspace at <path> and print the typed manifest.
    Check {
        /// Path to a workspace root (defaults to CWD).
        path: Option<PathBuf>,
    },
    /// Parse the workspace + report on the lockfile shape only.
    /// (Read-only diagnostic; for the transient-lock operator
    /// surface, see `gen lock`.)
    #[command(name = "lockfile-info")]
    LockfileInfo { path: Option<PathBuf> },
    /// Print the typed config at the given tier (bare / discovered /
    /// default / custom <yaml-path>).
    #[command(name = "config-show")]
    ConfigShow {
        #[arg(value_enum, default_value_t = TierArg::Default)]
        tier: TierArg,
        /// Path to a YAML overlay (only used when tier=custom).
        path: Option<PathBuf>,
    },
    /// Diff two tiers (each is bare / discovered / default).
    #[command(name = "config-diff")]
    ConfigDiff {
        #[arg(value_enum)]
        from: TierArg,
        #[arg(value_enum)]
        to: TierArg,
    },
    /// List the adapter routing table (filename → adapter name).
    Adapters,
    /// `gen quirks` — operator-visible view of every adapter's typed
    /// quirk registry. The canonical surface for "what does substrate
    /// know about?" — discoverable from one command per adapter.
    /// JSON output by default (machine + AST-renderer consumption).
    Quirks {
        /// Filter to a single adapter by name (e.g. `cargo`, `npm`).
        /// Default: every adapter the gen-cli was built against.
        #[arg(long)]
        adapter: Option<String>,
    },
    /// `gen dispatchers` — operator-visible reflection over every
    /// adapter's `#[derive(TypedDispatcher)]` enum. The substrate-
    /// wide "common language" surface: which typed variants exist
    /// per ecosystem, what fields each carries, what serde tag the
    /// runtime emits. Source of truth for the matching
    /// `substrate/lib/build/<eco>/quirk-apply.nix` helpers tables —
    /// any drift between this output and a helpers table is a CI
    /// failure.
    ///
    /// JSON output by default (machine + substrate-coverage-check
    /// consumption). Use `--ecosystem <name>` to filter.
    Dispatchers {
        /// Filter to a single ecosystem by name (e.g. `cargo`, `npm`).
        #[arg(long)]
        ecosystem: Option<String>,
        /// List entries from the gen-platform DispatcherCatalog
        /// (every Rust enum registered via gen_platform::register_dispatcher!)
        /// instead of the per-adapter reflection view. Surfaces the
        /// fleet-wide catalog — production dispatchers AND any
        /// non-adapter dispatcher a downstream crate registered.
        #[arg(long)]
        from_catalog: bool,
    },
    /// `gen dispatchers-skeleton` — emit substrate `quirk-apply.nix`
    /// skeleton for a registered dispatcher catalog entry. One Rust
    /// enum + one CLI call → typed Nix dispatch arm. Operator fills
    /// in the class-helper function bodies; the dispatch table is
    /// mechanical.
    #[command(name = "dispatchers-skeleton")]
    DispatchersSkeleton {
        /// Catalog label to emit (e.g. `gen.cargo.crate-quirk`).
        /// List available via `gen dispatchers --from-catalog`.
        #[arg(long)]
        label: String,
    },
    /// Render the workspace at <path> to Nix source (crate2nix shape).
    /// Output goes to `cfg.render.output_path` or stdout.
    Render { path: Option<PathBuf> },
    /// Generate Cargo.features.json sidecar (cargo-metadata-driven).
    /// Substrate's lockfile-builder reads this at eval to activate the
    /// right per-crate features. Tiny (~10-20% of Cargo.nix size).
    #[command(name = "lock-features")]
    LockFeatures { path: Option<PathBuf> },
    /// Generate the complete typed Cargo.build-spec.json.
    /// Composes Cargo.toml + Cargo.lock + cargo metadata into ONE
    /// typed JSON that substrate's lockfile-builder consumes directly.
    /// Replaces both Cargo.nix AND Cargo.features.json.
    #[command(name = "lock-build")]
    LockBuild { path: Option<PathBuf> },
    /// Canonical operator entrypoint. Regenerates every build sidecar
    /// the substrate consumer needs: Cargo.build-spec.json today,
    /// adapter-equivalents (npm/bundler/etc.) when they ship. Run
    /// whenever Cargo.lock or package-lock.json changes.
    ///
    /// One subcommand, one operation, deterministic output. The
    /// `lock-build` / `lock-features` subcommands remain as
    /// lower-level surfaces; `gen build` is what fleet operators
    /// invoke.
    Build {
        path: Option<PathBuf>,
        /// Per-platform cfg() filtering. When passed, cargo's resolver
        /// drops dep edges whose cfg(target_os/vendor/arch/family/…)
        /// don't match the given triple. Single-target output —
        /// committed specs that use this are stuck on one target.
        /// Prefer the default (multi-target) for committed specs and
        /// the IFD auto-regen path; reserve --filter-platform for
        /// per-CI-job artifact emission or debugging.
        ///
        /// Example: `--filter-platform=x86_64-unknown-linux-musl`.
        /// Mutually exclusive with --single-target.
        #[arg(long, value_name = "TRIPLE", conflicts_with = "single_target")]
        filter_platform: Option<String>,
        /// Opt OUT of multi-target emission (host-filtered single-
        /// target spec). The default is multi-target — one spec
        /// serves every fleet target, eliminating the gen-bootstrap
        /// chicken-and-egg. Pass --single-target only when you
        /// explicitly want a host-only spec (rare; usually wrong
        /// for committed specs).
        #[arg(long, conflicts_with = "filter_platform")]
        single_target: bool,
        /// Fast-path: if the committed spec's cargo_lock_hash matches
        /// the current Cargo.lock's BLAKE3 digest, skip regeneration
        /// entirely. Idempotent on the no-drift path — cost drops
        /// from "full cargo metadata + serde emit" to "two file
        /// reads + two hashes." The fleet sweep (fleet/rebuild)
        /// turns this on by default; manual `gen build` calls leave
        /// it off so the operator can force a regen on demand.
        #[arg(long)]
        if_stale: bool,
        /// Atomically commit the regenerated spec to git. Useful for
        /// bootstrap-exception repos (gen, engenho with private deps)
        /// where the spec is intentionally committed and freshness is
        /// maintained at the operator boundary. Combined with `cargo
        /// update`, the operator's workflow becomes:
        ///
        ///   cargo update && gen build . --commit
        ///
        /// The commit lands as `gen-spec-bot` so the regen is
        /// distinct from operator-authored commits. No-op when the
        /// spec hasn't drifted (idempotent).
        #[arg(long)]
        commit: bool,
        /// Resolve the spec with `--no-default-features`.
        ///
        /// gen bakes the RESOLVED feature set into the spec, and Nix's
        /// `rootFeatures` is not honored on the lockfile-builder path —
        /// so the spec IS the feature decision, and there is no way to
        /// ask a generated spec for FEWER features than it was generated
        /// with. A crate that needs two variants needs two specs.
        ///
        /// Requires --out: writing a reduced-feature spec over the
        /// canonical one would silently change the primary build, which
        /// is the exact failure this flag exists to make impossible.
        #[arg(long, requires = "out")]
        no_default_features: bool,
        /// Features to activate, comma-separated. Composes with
        /// --no-default-features and with the manifest's
        /// `[package.metadata.pleme].features` opt-ins.
        #[arg(long, value_name = "LIST", value_delimiter = ',', requires = "out")]
        features: Vec<String>,
        /// Write the spec here instead of `Cargo.build-spec.json`.
        ///
        /// Relative paths resolve against the workspace root. A variant
        /// spec gets no `Cargo.gen.lock` delta — that tie names THE spec,
        /// and two ties would leave the D2 pre-commit check unable to say
        /// which one it is judging.
        #[arg(long, value_name = "PATH")]
        out: Option<PathBuf>,
    },
    /// Typed read-only freshness check of `Cargo.build-spec.json`.
    /// Returns the `Freshness` variants (Fresh / Drifted /
    /// UnhashedSpec / MissingSpec / MissingLock) without writing
    /// anything. Exit code is non-zero when `needs_regen()` is true
    /// so shell pipelines can branch on it cheaply.
    ///
    /// `gen check-spec` is the typed companion to `gen build
    /// --if-stale` — same algorithm, different surface (read-only
    /// report vs in-place fast-path).
    #[command(name = "check-spec")]
    CheckSpec {
        /// Path to a workspace root (defaults to CWD).
        path: Option<PathBuf>,
    },
    /// `gen confirm` — pure OFFLINE delta-freshness gate. Reads ONLY
    /// `Cargo.lock` + the committed `Cargo.gen.lock`: recomputes
    /// `sha256(Cargo.lock)` and compares it to the delta's recorded
    /// `cargo_lock_sha256` (the D2 freshness tie). Exit code is non-zero
    /// when the delta is STALE (Cargo.lock changed without a `gen build`),
    /// missing, or untied. No `cargo metadata`, no network, no `~/.cargo`
    /// — safe inside a Nix `nix flake check` sandbox.
    ///
    /// This is the offline DESTINATION of substrate's `gen-confirm` check:
    /// `gen check` proves the workspace PARSES; `gen confirm` proves the
    /// committed delta is FRESH against its lock. (Distinct from the
    /// `Adapter::confirm` trait method, which regenerates the spec via
    /// `cargo metadata` and cannot run offline.)
    Confirm {
        /// Path to a workspace root (defaults to CWD).
        path: Option<PathBuf>,
        /// Gate only a delta that EXISTS but is stale/untied; TOLERATE a
        /// repo that commits no `Cargo.gen.lock` (the substrate IFD
        /// reconstruction path). This is the mode substrate's `gen-confirm`
        /// nix-flake-check uses, so consumers not on the delta-only
        /// doctrine are not regressed. Default (unset) is strict: a missing
        /// delta also fails.
        #[arg(long)]
        if_present: bool,
    },
    /// `gen fleet-check` — recursively report freshness for every
    /// workspace under <path>. Same shape as `gen fleet-sweep` but
    /// read-only: emits typed `Freshness` per repo without
    /// touching any spec. Exit code is non-zero iff any repo's
    /// `needs_regen()` is true.
    #[command(name = "fleet-check")]
    FleetCheck {
        /// Root directory containing N repo sub-directories.
        path: PathBuf,
    },
    /// Run gen build across every cargo workspace under <path>.
    /// Emits a typed sweep report (JSON|YAML). Use --write to persist
    /// Cargo.build-spec.json into each successful repo for committing.
    #[command(name = "fleet-sweep")]
    FleetSweep {
        /// Root directory containing N repo sub-directories.
        path: PathBuf,
        /// Persist Cargo.build-spec.json into each successful repo.
        /// Default false — dry-run sweep for fleet-health visibility.
        #[arg(long)]
        write: bool,
    },
    /// `gen flake-lint` — detect stale `inputs.X.follows = "Y"` overrides
    /// where the consumer flake doesn't declare `Y`. Pure-eval typed
    /// surface; substrate's auto-release CI runs this as a gate.
    /// Exit code is non-zero iff any issue is reported.
    #[command(name = "flake-lint")]
    FlakeLint {
        /// Path to the flake's directory (must contain `flake.nix`).
        /// Defaults to CWD.
        path: Option<PathBuf>,
        /// Rewrite the flake.nix to remove the offending `.follows`
        /// lines. Idempotent — safe to re-run.
        #[arg(long)]
        fix: bool,
    },
    /// `gen lock` — operator-facing surface for the transient-lock
    /// pattern. Default (no mode flag): print the typed
    /// `LockLifecycleState` (read-only). With `--snapshot`, write a
    /// fresh `Cargo.build-spec.json` as an explicit release-time pin.
    /// With `--update`, regenerate + emit a typed `LockDiff` against
    /// the previously-committed spec. With `--reset`, delete the
    /// committed spec (return to transient `Unlocked` state).
    #[command(name = "lock")]
    Lock {
        /// Workspace root (defaults to CWD).
        path: Option<PathBuf>,
        /// Snapshot: write Cargo.build-spec.json as a deliberate
        /// committable artifact (operator opts into the Locked state).
        #[arg(long, conflicts_with_all = ["update", "reset"])]
        snapshot: bool,
        /// Regenerate from current Cargo.lock + emit typed LockDiff
        /// (added / removed / upgraded / source-changed) against the
        /// previously-committed spec. Writes the new spec on success.
        #[arg(long, conflicts_with_all = ["snapshot", "reset"])]
        update: bool,
        /// Delete the committed Cargo.build-spec.json — return to
        /// transient (`Unlocked`) state. Substrate IFD will regen on
        /// next build.
        #[arg(long, conflicts_with_all = ["snapshot", "update"])]
        reset: bool,
    },
    /// Commit (and optionally push) Cargo.build-spec.json across the
    /// fleet. Walks repos under <path>; for each, stages only the
    /// sidecar (never `git add -A`) and commits with the canonical
    /// deterministic message. Idempotent — repos already at-HEAD are
    /// skipped.
    #[command(name = "fleet-commit")]
    FleetCommit {
        path: PathBuf,
        /// Push after each commit. Default false.
        #[arg(long)]
        push: bool,
        /// `git pull --rebase` before push to absorb upstream commits
        /// (CI auto-release lands frequently; this avoids
        /// non-fast-forward rejections).
        #[arg(long)]
        rebase_first: bool,
    },
    /// Migrate Rust repos to the DELTA-ONLY spec doctrine from a typed
    /// tatara-lisp plan: regenerate, commit Cargo.gen.lock, retire
    /// (git rm + gitignore) Cargo.build-spec.json. Typed per-repo
    /// outcomes; never `git add -A`; divergence-aware push.
    #[command(name = "fleet-migrate")]
    FleetMigrate {
        /// Path to the `(fleet-migration-plan …)` `.tatara.lisp` file.
        plan: PathBuf,
        /// Override the plan: build + commit only, do not push.
        #[arg(long)]
        no_push: bool,
        /// Override the plan's concurrency (repos migrated in parallel).
        #[arg(long)]
        jobs: Option<usize>,
        /// Refresh stale pleme-io git pins (`cargo update`) before building
        /// — heals GC'd-rev `branch=main` hangs.
        #[arg(long)]
        refresh: bool,
    },
    /// Verify (read-only) that a plan's repos reached the delta-only end
    /// state on their remote branch. Typed per-repo state; no mutation.
    #[command(name = "fleet-verify")]
    FleetVerify {
        /// Path to the `(fleet-migration-plan …)` `.tatara.lisp` file.
        plan: PathBuf,
        /// `git fetch origin <branch>` before reading (authoritative).
        #[arg(long)]
        fetch: bool,
        /// Concurrency override.
        #[arg(long)]
        jobs: Option<usize>,
    },
    /// Probe the configured substituters for every lockfile entry in
    /// the workspace at <path>. Report hit / miss / hit-rate.
    #[command(name = "cache-probe")]
    CacheProbe { path: Option<PathBuf> },
    /// Emit a canonical pleme-io repository scaffold: 4-line flake.nix
    /// against substrate.<ecosystem>.<shape>, minimal manifest +
    /// source, gitignore, auto-release CI shim. One command → full
    /// repo bootstrap.
    Scaffold {
        /// Shape: tool | workspace | library | service | binary.
        #[arg(value_enum)]
        shape: ScaffoldShape,
        /// Name (used for the crate / package name + binary name).
        name: String,
        /// Target directory (defaults to ./<name>).
        #[arg(long, short = 'd')]
        dir: Option<PathBuf>,
        /// Owner/org for github URLs (defaults to "pleme-io").
        #[arg(long, default_value = "pleme-io")]
        owner: String,
    },
    /// `gen scaffold-adapter` — stamp a new gen-<ecosystem> adapter
    /// crate skeleton inside the gen workspace. Materializes the
    /// seven-artifact intake pattern (theory/ECOSYSTEM-INTAKE.md):
    /// Cargo.toml + adapter.rs + build_spec.rs + quirks.rs +
    /// invariants.rs + error.rs + lib.rs, with the `Spec` +
    /// `QuirkRegistry` + `Invariants` derives wired and stub
    /// Adapter impls returning `Unsupported`. Author then fills in
    /// `Adapter::build`, registers the crate in the workspace, and
    /// the trait surface lights up.
    ///
    /// Pillar 12 (generation over composition) applied to gen itself.
    #[command(name = "scaffold-adapter")]
    ScaffoldAdapter {
        /// Ecosystem name: npm | pip | gomod | helm | ansible | …
        /// Becomes the crate dir name (`gen-<name>`) + the adapter
        /// `name()` return value.
        name: String,
        /// The manifest filename the new adapter detects on
        /// (`package.json` for npm, `pyproject.toml` for pip, etc).
        #[arg(long)]
        manifest: String,
        /// Target dir for the new crate (default: `crates/gen-<name>`).
        #[arg(long)]
        dir: Option<PathBuf>,
    },
    /// `gen kanshou` — operator-facing reflection over every
    /// introspectable process on this host. Subcommands list live
    /// consumers, dump their schema, and ship typed queries through
    /// the kanshou Unix socket. The substrate-side operator surface
    /// for the kanshou wave.
    /// `gen adopt-go` — the Go adoption census. Scans module roots and
    /// reports how many are ELIGIBLE for gen adoption, with every refusal
    /// carrying a named reason.
    ///
    /// Measures; adopts nothing. `--dry-run` is the only mode that exists,
    /// so there is no write path to reach by accident.
    ///
    /// The classification is `gen_gomod::directive` — byte-for-byte the same
    /// predicate substrate evaluates in Nix at build time, bound to one
    /// shared vector table. A census that disagreed with the build gate
    /// about "eligible" would be worse than no census.
    #[command(name = "adopt-go")]
    AdoptGo {
        /// Module roots to scan (recursively). Defaults to CWD.
        #[arg(long = "root")]
        roots: Vec<PathBuf>,
        /// The fleet Go toolchain to classify against.
        #[arg(long, default_value = adopt_cli::DEFAULT_FLEET_GO)]
        fleet_go: String,
        /// Required. The only mode that exists — see the module docs.
        #[arg(long, required = true)]
        dry_run: bool,
        /// Emit JSON instead of the human report.
        #[arg(long)]
        json: bool,
    },
    #[command(subcommand)]
    Kanshou(KanshouCmd),
}

#[derive(Subcommand, Debug)]
enum KanshouCmd {
    /// Enumerate every introspectable process on this host. Walks
    /// `$HOME/Library/Application Support/kanshou` (darwin) /
    /// `$XDG_RUNTIME_DIR/kanshou` (linux) for the per-(app, pid)
    /// sockets and prints them as JSON.
    List {
        /// Filter to a single app name (e.g. `mado`, `frost`).
        #[arg(long)]
        app: Option<String>,
    },
    /// Ship a typed query against a live consumer. Path is a
    /// dot-separated leaf list — `frame_perf`, `sessions.count`,
    /// `vt.pending_responses`. By default the most-recent PID for
    /// the named app is targeted; `--pid` overrides.
    Query {
        /// App name to target (matches `kanshou::discover(Some(app))`).
        app: String,
        /// Dot-separated path into the consumer's AppState.
        path: String,
        /// Target a specific PID rather than the most-recent one.
        #[arg(long)]
        pid: Option<u32>,
    },
    /// Print the consumer's declared schema — the top-level field
    /// list returned by `Introspect::schema()`. Useful for "what
    /// can I query on this app?" reconnaissance.
    Schema {
        app: String,
        #[arg(long)]
        pid: Option<u32>,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum ScaffoldShape {
    Tool,
    Workspace,
    Library,
    Service,
    Binary,
}

impl ScaffoldShape {
    fn as_str(self) -> &'static str {
        match self {
            Self::Tool => "tool",
            Self::Workspace => "workspace",
            Self::Library => "library",
            Self::Service => "service",
            Self::Binary => "binary",
        }
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum TierArg {
    Bare,
    Discovered,
    Default,
    Custom,
}

impl TierArg {
    fn into_shikumi(self, custom_path: Option<PathBuf>) -> ConfigTier {
        match self {
            Self::Bare => ConfigTier::Bare,
            Self::Discovered => ConfigTier::Discovered,
            Self::Default => ConfigTier::Default,
            Self::Custom => {
                ConfigTier::Custom(custom_path.unwrap_or_else(|| PathBuf::from(".gen.yaml")))
            }
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let cfg = GenConfig::resolve_from_env("GEN_TIER");
    match run(&cli, &cfg) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("gen: error: {e}");
            ExitCode::from(1)
        }
    }
}

fn run(cli: &Cli, cfg: &GenConfig) -> Result<(), CliError> {
    match &cli.cmd {
        Cmd::Check { path } => {
            let root = resolve_root(path, cfg);
            let manifest = dispatch(&root, cfg)?;
            emit(&manifest, cli.format)
        }
        Cmd::LockfileInfo { path } => {
            let root = resolve_root(path, cfg);
            let manifest = dispatch(&root, cfg)?;
            #[derive(serde::Serialize)]
            struct LockReport<'a> {
                has_lockfile: bool,
                resolved_count: usize,
                content_addressed_hash: Option<String>,
                content_addressed_sources: usize,
                first_few: Vec<&'a str>,
            }
            let report = match &manifest.lockfile {
                None => LockReport {
                    has_lockfile: false,
                    resolved_count: 0,
                    content_addressed_hash: None,
                    content_addressed_sources: 0,
                    first_few: Vec::new(),
                },
                Some(l) => {
                    let cac = l
                        .resolved
                        .values()
                        .filter(|r| r.source.is_content_addressed())
                        .count();
                    LockReport {
                        has_lockfile: true,
                        resolved_count: l.resolved.len(),
                        content_addressed_hash: Some(l.content_addressed_hash.hex()),
                        content_addressed_sources: cac,
                        first_few: l.resolved.keys().take(10).map(String::as_str).collect(),
                    }
                }
            };
            emit(&report, cli.format)
        }
        Cmd::ConfigShow { tier, path } => {
            let resolved = GenConfig::resolve_tier(tier.into_shikumi(path.clone()));
            emit(&resolved, cli.format)
        }
        Cmd::ConfigDiff { from, to } => {
            let a = GenConfig::resolve_tier(from.into_shikumi(None));
            let b = GenConfig::resolve_tier(to.into_shikumi(None));
            let diff = b.diff_against(&a);
            println!("{}", diff.render_unified());
            Ok(())
        }
        Cmd::Render { path } => {
            let root = resolve_root(path, cfg);
            let adapter = pick_adapter(&root, cfg)?;
            if adapter != "cargo" {
                return Err(CliError::RenderNotImplementedForAdapter(adapter));
            }
            let mut manifest = dispatch(&root, cfg)?;
            // Enrich every ResolvedPackage with `links` from
            // Cargo.build-spec.json (when present). Without this,
            // buildRustCrate omits `links = "<symbol>"` and ring /
            // openssl-sys / bzip2-sys / libsqlite3-sys / libz-sys / …
            // build scripts assert-panic on the missing
            // CARGO_MANIFEST_LINKS env var. The build-spec is the
            // typed source of truth; this overlay is the only thing
            // that gets the data into the Nix output.
            enrich_manifest_with_build_spec(&root, &mut manifest);
            let text = gen_nix::render_workspace_to_cargo_nix(&manifest);
            if cfg.render.output_path.is_empty() {
                println!("{text}");
            } else {
                std::fs::write(&cfg.render.output_path, &text).map_err(CliError::Io)?;
                eprintln!(
                    "gen: wrote {} bytes to {}",
                    text.len(),
                    cfg.render.output_path
                );
            }
            Ok(())
        }
        Cmd::LockFeatures { path } => {
            let root = resolve_root(path, cfg);
            let out = gen_cargo::features::generate_and_write(&root).map_err(CliError::Cargo)?;
            eprintln!("gen: wrote {}", out.display());
            Ok(())
        }
        Cmd::LockBuild { path } => {
            let root = resolve_root(path, cfg);
            let out = gen_cargo::build_spec::generate_and_write(&root).map_err(CliError::Cargo)?;
            eprintln!("gen: wrote {}", out.display());
            Ok(())
        }
        Cmd::FleetCommit {
            path,
            push,
            rebase_first,
        } => {
            let report = gen_cargo::fleet_commit::run(path, *push, *rebase_first)
                .map_err(CliError::Cargo)?;
            #[derive(serde::Serialize)]
            struct Summary {
                root: std::path::PathBuf,
                total: usize,
                committed: usize,
                pushed: usize,
                skipped: usize,
                failed: usize,
                elapsed_ms: u64,
                report: gen_cargo::fleet_commit::CommitReport,
            }
            let summary = Summary {
                root: report.root.clone(),
                total: report.total(),
                committed: report.committed_count(),
                pushed: report.pushed_count(),
                skipped: report.skipped_count(),
                failed: report.failed_count(),
                elapsed_ms: report.total_elapsed_ms,
                report,
            };
            emit(&summary, cli.format)
        }
        Cmd::FleetMigrate {
            plan,
            no_push,
            jobs,
            refresh,
        } => {
            let src = std::fs::read_to_string(plan)
                .map_err(|e| CliError::Other(format!("read {}: {e}", plan.display())))?;
            let report = fleet_migrate_plan::run_plan(&src, *no_push, *jobs, *refresh)
                .map_err(CliError::Other)?;
            #[derive(serde::Serialize)]
            struct Summary {
                total: usize,
                migrated: usize,
                pushed: usize,
                skipped: usize,
                failed: usize,
                elapsed_ms: u64,
                report: gen_cargo::fleet_migrate::MigrateReport,
            }
            let summary = Summary {
                total: report.total(),
                migrated: report.migrated_count(),
                pushed: report.pushed_count(),
                skipped: report.skipped_count(),
                failed: report.failed_count(),
                elapsed_ms: report.total_elapsed_ms,
                report,
            };
            emit(&summary, cli.format)
        }
        Cmd::FleetVerify { plan, fetch, jobs } => {
            let src = std::fs::read_to_string(plan)
                .map_err(|e| CliError::Other(format!("read {}: {e}", plan.display())))?;
            let report =
                fleet_migrate_plan::verify_plan(&src, *fetch, *jobs).map_err(CliError::Other)?;
            #[derive(serde::Serialize)]
            struct Summary {
                total: usize,
                delta_only: usize,
                all_delta_only: bool,
                elapsed_ms: u64,
                report: gen_cargo::fleet_verify::VerifyReport,
            }
            let summary = Summary {
                total: report.rows.len(),
                delta_only: report.delta_only(),
                all_delta_only: report.all_delta_only(),
                elapsed_ms: report.total_elapsed_ms,
                report,
            };
            emit(&summary, cli.format)
        }
        Cmd::FleetSweep { path, write } => {
            let report = gen_cargo::fleet_sweep::run(path, *write).map_err(CliError::Cargo)?;
            #[derive(serde::Serialize)]
            struct Summary {
                root: std::path::PathBuf,
                total: usize,
                ok: usize,
                failed: usize,
                skipped: usize,
                total_spec_bytes: usize,
                elapsed_ms: u64,
                failures_by_category: indexmap::IndexMap<String, Vec<String>>,
                #[serde(skip_serializing_if = "std::ops::Not::not")]
                wrote_sidecars: bool,
                report: gen_cargo::fleet_sweep::SweepReport,
            }
            let by_cat = report.failures_by_category();
            let by_cat_str: indexmap::IndexMap<String, Vec<String>> = by_cat
                .into_iter()
                .map(|(k, v)| (format!("{k:?}"), v))
                .collect();
            let summary = Summary {
                root: report.root.clone(),
                total: report.total(),
                ok: report.ok_count(),
                failed: report.failed_count(),
                skipped: report.skipped_count(),
                total_spec_bytes: report.total_spec_bytes(),
                elapsed_ms: report.total_elapsed_ms,
                failures_by_category: by_cat_str,
                wrote_sidecars: *write,
                report,
            };
            emit(&summary, cli.format)
        }
        Cmd::Build {
            path,
            filter_platform,
            single_target,
            if_stale,
            commit,
            no_default_features,
            features,
            out,
        } => {
            let root = resolve_root(path, cfg);
            // Atomic: regenerate every sidecar the substrate consumer
            // needs based on the adapter the workspace declares. Today
            // that's BuildSpec for cargo workspaces; future adapters
            // land here as they ship.
            //
            // Default is MULTI-TARGET (commits one spec for every fleet
            // target — eliminates gen-bootstrap chicken-and-egg). Opt
            // out only with --single-target or --filter-platform.
            let adapter = pick_adapter(&root, cfg)?;
            match adapter.as_str() {
                "cargo" => {
                    // --if-stale: fast-path through the spec's
                    // cargo_lock_hash; skip regen if hash matches.
                    // Mutually exclusive with single-target /
                    // filter-platform (those force a full regen by
                    // construction).
                    if *if_stale && filter_platform.is_none() && !*single_target {
                        let (freshness, out) =
                            gen_cargo::build_spec::generate_and_write_if_stale(&root)
                                .map_err(CliError::Cargo)?;
                        eprintln!(
                            "gen build --if-stale: {} ({})",
                            freshness.summary(),
                            out.display()
                        );
                        return Ok(());
                    }
                    let out = if let Some(triple) = filter_platform {
                        gen_cargo::build_spec::generate_for_target_and_write(&root, &triple)
                            .map_err(CliError::Cargo)?
                    } else if *single_target {
                        gen_cargo::build_spec::generate_and_write(&root).map_err(CliError::Cargo)?
                    } else {
                        let selection = gen_cargo::build_spec::FeatureSelection {
                            no_default_features: *no_default_features,
                            features: features.clone(),
                        };
                        gen_cargo::build_spec::generate_multi_target_and_write_to(
                            &root,
                            &selection,
                            out.as_deref(),
                        )
                        .map_err(CliError::Cargo)?
                    };
                    eprintln!("gen build: wrote {}", out.display());
                    if *commit {
                        commit_spec_if_drifted(&root)?;
                    }
                }
                "gomod" => {
                    // gomod M1 is a single-tuple per-package incremental
                    // encoder. It has no Cargo.lock-hash fast-path, so
                    // --if-stale / --single-target / --commit are cargo-
                    // only and don't apply here; --filter-platform (a Rust
                    // triple) selects the Go (goos, goarch) tuple, else the
                    // build host. Emits Go.build-spec.json next to go.mod.
                    let out = match filter_platform {
                        Some(triple) => gen_gomod::generate_for_target_and_write(&root, triple)
                            .map_err(CliError::Gomod)?,
                        None => gen_gomod::generate_and_write(&root).map_err(CliError::Gomod)?,
                    };
                    eprintln!("gen build: wrote {}", out.display());
                }
                other => {
                    return Err(CliError::RenderNotImplementedForAdapter(other.to_string()));
                }
            }
            Ok(())
        }
        Cmd::CheckSpec { path } => {
            let root = resolve_root(path, cfg);
            let adapter = pick_adapter(&root, cfg)?;
            match adapter.as_str() {
                "cargo" => {
                    let freshness = gen_cargo::build_spec::check_freshness(&root);
                    let needs_regen = freshness.needs_regen();
                    emit(&freshness, cli.format)?;
                    // Non-zero exit code when regen is needed so
                    // shell pipelines branch cheaply: `gen
                    // check-spec && echo fresh || gen build`.
                    if needs_regen {
                        std::process::exit(1);
                    }
                    Ok(())
                }
                other => {
                    return Err(CliError::RenderNotImplementedForAdapter(other.to_string()));
                }
            }
        }
        Cmd::Confirm { path, if_present } => {
            let root = resolve_root(path, cfg);
            let adapter = pick_adapter(&root, cfg)?;
            match adapter.as_str() {
                "cargo" => {
                    // PURE offline check: reads only Cargo.lock +
                    // Cargo.gen.lock. Never `Adapter::confirm` (that
                    // regenerates the spec via `cargo metadata` — no cargo,
                    // no network in the check sandbox).
                    let freshness = gen_cargo::gen_delta::confirm_freshness(&root);
                    // Default = strict (a missing delta fails); `--if-present`
                    // = gate only an existing-but-stale delta (no regression
                    // for IFD-path consumers committing no delta).
                    let gate = freshness.gates_failure(!*if_present);
                    emit(&freshness, cli.format)?;
                    if gate {
                        std::process::exit(1);
                    }
                    Ok(())
                }
                other => {
                    return Err(CliError::RenderNotImplementedForAdapter(other.to_string()));
                }
            }
        }
        Cmd::FleetCheck { path } => {
            #[derive(serde::Serialize)]
            struct RepoFreshness<'a> {
                path: &'a str,
                freshness: gen_cargo::build_spec::Freshness,
            }
            // Enumerate repos = top-level subdirectories that have a
            // `Cargo.toml` at their root. Same shape as fleet-sweep
            // but read-only.
            let mut entries: Vec<RepoFreshness> = Vec::new();
            let mut owned_paths: Vec<String> = Vec::new();
            let mut needs_any_regen = false;
            for entry in std::fs::read_dir(path).map_err(|e| {
                CliError::Other(format!("fleet-check: read_dir {}: {e}", path.display()))
            })? {
                let Ok(entry) = entry else { continue };
                let p = entry.path();
                if !p.is_dir() {
                    continue;
                }
                if !p.join("Cargo.toml").is_file() {
                    continue;
                }
                let f = gen_cargo::build_spec::check_freshness(&p);
                if f.needs_regen() {
                    needs_any_regen = true;
                }
                owned_paths.push(p.display().to_string());
                entries.push(RepoFreshness {
                    path: "",
                    freshness: f,
                });
            }
            // Stitch the borrowed `path` field into each entry — we
            // can't borrow from `owned_paths` while pushing to it, so
            // build the array of borrowed pairs after the walk.
            let final_entries: Vec<RepoFreshness> = entries
                .into_iter()
                .zip(owned_paths.iter())
                .map(|(e, p)| RepoFreshness {
                    path: p.as_str(),
                    freshness: e.freshness,
                })
                .collect();
            emit(&final_entries, cli.format)?;
            if needs_any_regen {
                std::process::exit(1);
            }
            Ok(())
        }
        Cmd::FlakeLint { path, fix } => {
            let root = path
                .clone()
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
            let source = flake_lint::NixCliMetadataSource;
            let report = flake_lint::run(&source, &root, *fix)
                .map_err(|e| CliError::Other(format!("flake-lint: {e}")))?;
            emit(&report, cli.format)?;
            if !report.issues.is_empty() && !*fix {
                std::process::exit(1);
            }
            Ok(())
        }
        Cmd::Lock {
            path,
            snapshot,
            update,
            reset,
        } => {
            // Dispatch through the typed `LockLifecyclePrimitive`
            // trait. Future adapters (gen-npm, gen-bundler, …) get
            // this entire surface for free by impl'ing the trait —
            // the CLI handler stays adapter-agnostic.
            use gen_cargo::lock_lifecycle::{self, CargoLifecycle, LockLifecycleState};
            use gen_types::LockLifecyclePrimitive;
            let root = path
                .clone()
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
            let prim = CargoLifecycle::new();
            let state = prim.current_state(&root);
            let spec_path = root.join("Cargo.build-spec.json");

            #[derive(serde::Serialize)]
            struct LockReport<'a> {
                root: &'a std::path::Path,
                action: &'a str,
                state_before: &'a LockLifecycleState,
                #[serde(skip_serializing_if = "Option::is_none")]
                state_after: Option<LockLifecycleState>,
                #[serde(skip_serializing_if = "Option::is_none")]
                diff: Option<lock_lifecycle::LockDiff>,
            }

            let _ = spec_path; // computed inside the trait impls
            match (*snapshot, *update, *reset) {
                (false, false, false) => {
                    // Default: read-only — print state.
                    let r = LockReport {
                        root: &root,
                        action: "show",
                        state_before: &state,
                        state_after: None,
                        diff: None,
                    };
                    emit(&r, cli.format)?;
                    if prim.requires_operator_action(&state) {
                        std::process::exit(1);
                    }
                    Ok(())
                }
                (true, false, false) => {
                    prim.snapshot(&root)
                        .map_err(|e| CliError::Other(format!("gen lock --snapshot: {e}")))?;
                    let after = prim.current_state(&root);
                    let r = LockReport {
                        root: &root,
                        action: "snapshot",
                        state_before: &state,
                        state_after: Some(after),
                        diff: None,
                    };
                    emit(&r, cli.format)
                }
                (false, true, false) => {
                    let diff = prim
                        .update(&root)
                        .map_err(|e| CliError::Other(format!("gen lock --update: {e}")))?;
                    let after = prim.current_state(&root);
                    let r = LockReport {
                        root: &root,
                        action: "update",
                        state_before: &state,
                        state_after: Some(after),
                        diff: Some(diff),
                    };
                    emit(&r, cli.format)
                }
                (false, false, true) => {
                    prim.reset(&root)
                        .map_err(|e| CliError::Other(format!("gen lock --reset: {e}")))?;
                    let after = prim.current_state(&root);
                    let r = LockReport {
                        root: &root,
                        action: "reset",
                        state_before: &state,
                        state_after: Some(after),
                        diff: None,
                    };
                    emit(&r, cli.format)
                }
                _ => unreachable!("clap conflicts_with_all prevents multi-mode"),
            }
        }
        Cmd::CacheProbe { path } => {
            let root = resolve_root(path, cfg);
            let manifest = dispatch(&root, cfg)?;
            let client = gen_cache_attic::client_from_config(&cfg.cache);
            let keys: Vec<gen_cache_attic::CacheKey> = match &manifest.lockfile {
                None => Vec::new(),
                Some(l) => l
                    .resolved
                    .values()
                    .filter(|r| r.source.is_content_addressed())
                    .map(|r| {
                        gen_cache_attic::CacheKey::new(
                            // Use the integrity hash bytes when available;
                            // fall back to BLAKE3 of the dotted-id so every
                            // entry gets a stable typed key.
                            r.integrity
                                .as_deref()
                                .and_then(|i| {
                                    let hex = i.strip_prefix("sha256:").unwrap_or(i);
                                    gen_types::ContentHash::from_hex_padded(hex)
                                })
                                .unwrap_or_else(|| {
                                    gen_types::ContentHash::of(r.id.dotted().as_bytes())
                                }),
                            r.id.dotted(),
                        )
                    })
                    .collect(),
            };
            let report = client.probe_batch(&keys);
            #[derive(serde::Serialize)]
            struct Summary {
                substituters: Vec<String>,
                probed: usize,
                hits: usize,
                misses: usize,
                hit_rate: f64,
                report: gen_cache_attic::CacheReport,
            }
            let summary = Summary {
                substituters: cfg.cache.substituters.clone(),
                probed: report.outcomes.len(),
                hits: report.hit_count(),
                misses: report.miss_count(),
                hit_rate: report.hit_rate(),
                report,
            };
            emit(&summary, cli.format)
        }
        Cmd::Adapters => {
            #[derive(serde::Serialize)]
            struct AdapterRow<'a> {
                marker: &'a str,
                adapter: &'a str,
            }
            let rows: Vec<AdapterRow> = cfg
                .workspace
                .adapter_routing
                .iter()
                .map(|(k, v)| AdapterRow {
                    marker: k.as_str(),
                    adapter: v.as_str(),
                })
                .collect();
            emit(&rows, cli.format)
        }
        Cmd::Scaffold {
            shape,
            name,
            dir,
            owner,
        } => scaffold_repo(*shape, name, dir.as_deref(), owner)
            .map_err(|e| CliError::Other(format!("scaffold: {e}"))),
        Cmd::ScaffoldAdapter {
            name,
            manifest,
            dir,
        } => {
            scaffold_adapter(name, manifest, dir.as_deref())
                .map_err(|e| CliError::Other(format!("scaffold-adapter: {e}")))?;
            println!(
                "✓ gen-{name} scaffolded.\n  Next: add `\"crates/gen-{name}\",` to the gen workspace members in `Cargo.toml`, fill in adapter::Adapter::build, then `cargo build -p gen-{name}`."
            );
            Ok(())
        }
        Cmd::AdoptGo {
            roots,
            fleet_go,
            dry_run,
            json,
        } => {
            // `required = true` already refuses the flagless form; this asserts
            // the invariant at the point it is relied on rather than trusting a
            // clap attribute to stay put.
            debug_assert!(*dry_run, "adopt-go has no non-dry-run mode");
            let roots = if roots.is_empty() {
                vec![PathBuf::from(".")]
            } else {
                roots.clone()
            };
            let code = adopt_cli::run(&adopt_cli::AdoptGoArgs {
                roots,
                fleet_go: fleet_go.clone(),
                json: *json,
            })
            .map_err(CliError::Other)?;
            if code != 0 {
                std::process::exit(code);
            }
            Ok(())
        }
        Cmd::Kanshou(sub) => {
            kanshou_dispatch(sub).map_err(|e| CliError::Other(format!("kanshou: {e}")))
        }
        Cmd::Dispatchers {
            ecosystem,
            from_catalog,
        } => {
            if *from_catalog {
                #[derive(serde::Serialize)]
                struct CatalogEntry<'a> {
                    label: &'a str,
                    variant_count: usize,
                    variant_kinds: Vec<&'static str>,
                    variant_fields: Vec<(&'static str, Vec<&'static str>)>,
                }
                let mut out: Vec<CatalogEntry> = gen_platform::catalog_registered()
                    .into_iter()
                    .map(|e| CatalogEntry {
                        label: e.label,
                        variant_count: (e.variant_count)(),
                        variant_kinds: (e.variant_kinds)(),
                        variant_fields: (e.variant_fields)(),
                    })
                    .collect();
                out.sort_by_key(|e| e.label);
                return emit(&out, cli.format);
            }
            #[derive(serde::Serialize)]
            struct EcoDispatcher<'a> {
                ecosystem: &'a str,
                variant_count: usize,
                variants: Vec<gen_types::DispatcherVariant>,
            }
            let filter = ecosystem.as_deref();
            let mut out: Vec<EcoDispatcher> = gen_types::registered_adapters()
                .iter()
                .filter(|a| filter.map(|f| a.name() == f).unwrap_or(true))
                .map(|a| {
                    let variants = a.dispatcher_reflection();
                    EcoDispatcher {
                        ecosystem: a.name(),
                        variant_count: variants.len(),
                        variants,
                    }
                })
                .collect();
            // Deterministic ordering for committed catalog snapshots
            // + reproducible diffing.
            out.sort_by_key(|e| e.ecosystem);
            emit(&out, cli.format)
        }
        Cmd::DispatchersSkeleton { label } => {
            let entry = gen_platform::catalog_by_label(label).ok_or_else(|| {
                CliError::Other(format!(
                    "no dispatcher registered with label '{label}'. Run `gen dispatchers --from-catalog` to list available labels."
                ))
            })?;
            print!("{}", gen_platform::to_helpers_skeleton(entry));
            Ok(())
        }
        Cmd::Quirks { adapter } => {
            // Distributed-slice introspection: every adapter
            // registers via `inventory::submit!` in its own crate;
            // gen-cli iterates the registered set without naming any
            // of them. Adding a new ecosystem requires zero edits
            // here — the new crate's `inventory::submit!` line is
            // sufficient.
            #[derive(serde::Serialize)]
            struct AdapterQuirks<'a> {
                adapter: &'a str,
                entries: Vec<gen_types::AdapterQuirkEntry>,
            }
            let filter = adapter.as_deref();
            let out: Vec<AdapterQuirks> = gen_types::registered_adapters()
                .iter()
                .filter(|a| filter.map(|f| a.name() == f).unwrap_or(true))
                .map(|a| AdapterQuirks {
                    adapter: a.name(),
                    entries: a.quirks_registry(),
                })
                .collect();
            emit(&out, cli.format)
        }
    }
}

/// Emit a canonical pleme-io repository scaffold. One command →
/// full bootstrap: 4-line flake.nix wired to `substrate.<ecosystem>.
/// <shape>`, minimal manifest, hello-world source, gitignore, and
/// the 3-line auto-release CI shim. Convention-named files;
/// operator edits start from a known-good baseline.
fn scaffold_repo(
    shape: ScaffoldShape,
    name: &str,
    dir: Option<&std::path::Path>,
    owner: &str,
) -> std::io::Result<()> {
    use std::fs;
    use std::path::PathBuf;

    let target = match dir {
        Some(p) => p.to_path_buf(),
        None => PathBuf::from(name),
    };
    fs::create_dir_all(&target)?;
    fs::create_dir_all(target.join("src"))?;
    fs::create_dir_all(target.join(".github").join("workflows"))?;

    // 4-line flake.nix.
    let flake_nix = format!(
        r#"{{
  description = "{name} — pleme-io {shape}";
  inputs.substrate.url = "github:pleme-io/substrate";
  outputs = {{ substrate, ... }}: substrate.rust.{shape} {{ src = ./.; }};
}}
"#,
        name = name,
        shape = shape.as_str(),
    );
    fs::write(target.join("flake.nix"), flake_nix)?;

    // Cargo.toml — shape-specific.
    let cargo_toml = match shape {
        ScaffoldShape::Library => format!(
            r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2024"
license = "MIT"
description = "{name} — pleme-io library"
repository = "https://github.com/{owner}/{name}"

[lib]
name = "{lib_name}"
"#,
            name = name,
            owner = owner,
            lib_name = name.replace('-', "_"),
        ),
        _ => format!(
            r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2024"
license = "MIT"
description = "{name} — pleme-io {shape}"
repository = "https://github.com/{owner}/{name}"

[[bin]]
name = "{name}"
path = "src/main.rs"
"#,
            name = name,
            owner = owner,
            shape = shape.as_str(),
        ),
    };
    fs::write(target.join("Cargo.toml"), cargo_toml)?;

    // Source file — main.rs (tool/binary/service/workspace) or lib.rs (library).
    let (src_path, src_body) = match shape {
        ScaffoldShape::Library => (
            "src/lib.rs",
            format!(
                "//! {name} — pleme-io library crate.\n\n#![deny(missing_docs)]\n\n/// Greeting helper.\npub fn hello() -> &'static str {{ \"hello from {name}\" }}\n",
                name = name,
            ),
        ),
        _ => (
            "src/main.rs",
            format!(
                "//! {name} — pleme-io {shape}.\n\nfn main() {{\n    println!(\"hello from {name}\");\n}}\n",
                name = name,
                shape = shape.as_str(),
            ),
        ),
    };
    fs::write(target.join(src_path), src_body)?;

    // .gitignore — Rust + Nix conventions.
    let gitignore = "/target\n/result\n.direnv/\n.envrc.cache\n";
    fs::write(target.join(".gitignore"), gitignore)?;

    // Auto-release CI shim per the pleme-io-auto-release skill.
    let workflow = r#"name: auto-release
on:
  push:
    branches: [main]
  workflow_dispatch:
    inputs:
      bump-type:
        description: "patch | minor | major"
        required: false
        default: patch
jobs:
  release:
    uses: pleme-io/substrate/.github/workflows/auto-release.yml@main
    with:
      bump-type: ${{ inputs.bump-type || 'patch' }}
    secrets: inherit
"#;
    fs::write(
        target
            .join(".github")
            .join("workflows")
            .join("auto-release.yml"),
        workflow,
    )?;

    eprintln!("gen scaffold: wrote {} files under {}", 5, target.display());
    Ok(())
}

fn resolve_root(arg: &Option<PathBuf>, cfg: &GenConfig) -> PathBuf {
    if let Some(p) = arg {
        return p.clone();
    }
    if !cfg.workspace.root.is_empty() {
        return PathBuf::from(&cfg.workspace.root);
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn emit<T: serde::Serialize>(value: &T, format: OutputFormat) -> Result<(), CliError> {
    let text = match format {
        OutputFormat::Json => serde_json::to_string_pretty(value).map_err(CliError::Json)?,
        OutputFormat::Yaml => serde_yaml::to_string(value).map_err(CliError::Yaml)?,
    };
    println!("{text}");
    Ok(())
}

#[derive(Debug, thiserror::Error)]
enum CliError {
    #[error(transparent)]
    Cargo(#[from] gen_cargo::CargoError),
    #[error(transparent)]
    Npm(#[from] gen_npm::NpmError),
    #[error(transparent)]
    Bundler(#[from] gen_bundler::BundlerError),
    #[error(transparent)]
    Gomod(#[from] gen_gomod::GomodError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Yaml(#[from] serde_yaml::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("no adapter found for workspace at {0} — checked: {1}")]
    NoAdapter(std::path::PathBuf, String),
    #[error("cargo workspace at {0} renders via Nix; npm/bundler renderers ship in M0.5b")]
    RenderNotImplementedForAdapter(String),
    #[error("{0}")]
    Other(String),
}

/// Overlay `links` field from `Cargo.build-spec.json` onto every
/// matching `ResolvedPackage` in the manifest. Silent no-op when the
/// build-spec file is missing or unparseable — the renderer falls back
/// to the existing (links-less) behaviour, matching gen's pre-fix
/// state for crates whose consumers haven't run `gen lock-build` yet.
///
/// The build-spec is keyed by the same `{name}-{version}` package_key
/// shape that gen-cargo uses internally, so lookup is exact.
fn enrich_manifest_with_build_spec(root: &std::path::Path, manifest: &mut gen_types::Manifest) {
    let path = root.join("Cargo.build-spec.json");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return;
    };
    let Ok(spec) = serde_json::from_str::<serde_json::Value>(&text) else {
        return;
    };
    let Some(crates_obj) = spec.get("crates").and_then(|v| v.as_object()) else {
        return;
    };

    // Build links lookup: package_key → links string. Only crates
    // that DECLARE links get an entry.
    // Index by BOTH the build-spec key `k` (which is `name-version-rev` for
    // git crates post-migration — TOOLCHAIN-FRESHNESS §X.4b.b) AND a
    // `name-version` alias, so the lockfile lookup below (which has only
    // name+version) still resolves a git crate's links.
    let links_by_key: std::collections::HashMap<String, String> = crates_obj
        .iter()
        .filter_map(|(k, v)| {
            let links = v.get("links").and_then(|l| l.as_str())?;
            let alias = match (
                v.get("name").and_then(|n| n.as_str()),
                v.get("version").and_then(|n| n.as_str()),
            ) {
                (Some(n), Some(ver)) => Some(format!("{n}-{ver}")),
                _ => None,
            };
            Some((k.clone(), alias, links.to_string()))
        })
        .flat_map(|(k, alias, links)| {
            let mut entries = vec![(k, links.clone())];
            if let Some(a) = alias {
                entries.push((a, links));
            }
            entries
        })
        .collect();
    if links_by_key.is_empty() {
        return;
    }

    let lockfile = match &mut manifest.lockfile {
        Some(l) => l,
        None => return,
    };
    for r in lockfile.resolved.values_mut() {
        let key = format!("{}-{}", r.id.name.as_str(), r.id.version);
        if let Some(links) = links_by_key.get(&key) {
            r.links = Some(links.clone());
        }
    }
}

/// Adapter dispatch — selects the matching parser based on the marker
/// file present at `root`. Honors `cfg.workspace.force_adapter` when
/// set, otherwise probes the routing table in declaration order.
fn dispatch(root: &std::path::Path, cfg: &GenConfig) -> Result<gen_types::Manifest, CliError> {
    let adapter = pick_adapter(root, cfg)?;
    match adapter.as_str() {
        "cargo" => Ok(gen_cargo::parse(root)?),
        "npm" => Ok(gen_npm::parse(root)?),
        "bundler" => Ok(gen_bundler::parse(root)?),
        other => Err(CliError::NoAdapter(
            root.to_path_buf(),
            format!("force_adapter={other} — not yet implemented"),
        )),
    }
}

fn pick_adapter(root: &std::path::Path, cfg: &GenConfig) -> Result<String, CliError> {
    if let Some(force) = &cfg.workspace.force_adapter {
        return Ok(force.clone());
    }
    let mut checked: Vec<String> = Vec::new();
    for (marker, adapter) in &cfg.workspace.adapter_routing {
        checked.push(marker.clone());
        if root.join(marker).exists() {
            return Ok(adapter.clone());
        }
    }
    Err(CliError::NoAdapter(root.to_path_buf(), checked.join(", ")))
}

fn _resolve_root_lint_silencer(p: &Path) -> PathBuf {
    p.to_path_buf()
}

/// Stamp a new gen-<ecosystem> adapter crate skeleton at the given
/// path. Materializes the seven-artifact intake pattern
/// (theory/ECOSYSTEM-INTAKE.md) — Cargo.toml + lib.rs + adapter.rs +
/// build_spec.rs + quirks.rs + invariants.rs + error.rs, with the
/// `Spec` + `QuirkRegistry` + `Invariants` derives pre-wired.
///
/// Adding the crate to the workspace `members` list + filling in
/// `Adapter::build` is the operator's only remaining work.
fn scaffold_adapter(name: &str, manifest: &str, dir: Option<&Path>) -> Result<(), std::io::Error> {
    use std::fs;
    let crate_dir = dir
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| Path::new("crates").join(format!("gen-{name}")));
    fs::create_dir_all(crate_dir.join("src"))?;
    let pascal = pascalize(name);
    let cargo_toml = format!(
        r#"[package]
name = "gen-{name}"
description = "gen — {name} adapter. Parses {manifest} + lockfile into a typed gen_types::Spec emission. One of N adapters that share the typed core via Spec/QuirkRegistry/Invariants traits."
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
homepage.workspace = true
repository.workspace = true
authors.workspace = true

[lib]
name = "gen_{snake}"
path = "src/lib.rs"

[lints]
workspace = true

[dependencies]
gen-types = {{ workspace = true }}
gen-macros = {{ path = "../gen-macros" }}
serde = {{ workspace = true }}
serde_json = {{ workspace = true }}
indexmap = {{ workspace = true }}
thiserror = {{ workspace = true }}
inventory = {{ workspace = true }}

[dev-dependencies]
indexmap = {{ workspace = true }}
"#,
        snake = name.replace('-', "_"),
    );
    fs::write(crate_dir.join("Cargo.toml"), cargo_toml)?;
    let lib_rs = format!(
        r#"//! `gen-{name}` — {name} adapter for the gen ecosystem.
//!
//! Parses `{manifest}` into a typed `BuildSpec` and emits it as
//! Cargo-equivalent typed JSON. See
//! `theory/ECOSYSTEM-INTAKE.md` for the seven-artifact contract.

pub mod adapter;
pub mod build_spec;
pub mod error;
pub mod invariants;
pub mod quirks;

pub use adapter::{pascal}Adapter;
pub use error::{{Result, {pascal}Error}};
"#,
    );
    fs::write(crate_dir.join("src/lib.rs"), lib_rs)?;
    let error_rs = format!(
        r#"use thiserror::Error;

#[derive(Debug, Error)]
pub enum {pascal}Error {{
    #[error("manifest not found: {{0}}")]
    ManifestNotFound(std::path::PathBuf),
    #[error("other: {{0}}")]
    Other(String),
}}

pub type Result<T> = std::result::Result<T, {pascal}Error>;
"#,
    );
    fs::write(crate_dir.join("src/error.rs"), error_rs)?;
    let build_spec_rs = format!(
        r#"//! Typed build spec for {name}. Implements `gen_types::Spec`
//! via `#[derive(SpecShape)]`.

use indexmap::IndexMap;
use serde::{{Deserialize, Serialize}};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize, gen_macros::SpecShape)]
#[spec(
    args = "PackageArgs",
    quirk = "crate::quirks::{pascal}Quirk",
    args_field = "args",
    root_field = "root_package",
    members_field = "workspace_members",
    crates_field = "packages"
)]
pub struct BuildSpec {{
    pub version: u32,
    pub packages: IndexMap<String, PackageSpec>,
    pub root_package: String,
    pub workspace_members: Vec<String>,
}}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PackageSpec {{
    pub name: String,
    pub version: String,
    pub args: PackageArgs,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quirks: Vec<crate::quirks::{pascal}Quirk>,
}}

/// Pre-shaped builder args for one package. Substrate spreads this
/// verbatim into the ecosystem's nixpkgs builder. Adapter authors
/// fill in fields matching `buildXxxPackage`'s mkArgs signature.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PackageArgs {{
    // TODO: add ecosystem-specific fields here.
}}
"#,
    );
    fs::write(crate_dir.join("src/build_spec.rs"), build_spec_rs)?;
    let quirks_rs = format!(
        r#"//! Typed quirk registry for {name}. Implements
//! `gen_types::QuirkRegistry` via `#[derive(QuirkRegistry)]`.
//!
//! Each registered entry names an upstream {name} package that needs
//! a known-good build-time workaround. The substrate consumer's
//! `{name}-quirk-apply.nix` dispatches mechanically on the variant
//! tags. Adding a new entry: append to `registry()` below.

use serde::{{Deserialize, Serialize}};

/// Typed quirks for known third-party upstream {name} packages.
/// Add variants as needed; remember to mirror with a Nix dispatch
/// arm in `substrate/lib/build/{name}/quirk-apply.nix`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum {pascal}Quirk {{
    // TODO: add ecosystem-specific quirk variants. Example shape:
    // ForceFlag {{ flag: String }},
}}

pub fn registry() -> Vec<(&'static str, Vec<{pascal}Quirk>)> {{
    // Hand-curated list of upstream-package quirks. Empty by
    // default; populate as the adapter encounters real bugs.
    Vec::new()
}}

#[derive(gen_macros::QuirkRegistry)]
#[quirks(enum_name = "{pascal}Quirk", registry_fn = "crate::quirks::registry")]
pub struct {pascal}Quirks;
"#,
    );
    fs::write(crate_dir.join("src/quirks.rs"), quirks_rs)?;
    let invariants_rs = format!(
        r#"//! Invariants over the {name} `BuildSpec`. Implements
//! `gen_types::Invariants` so cse-lint + gen confirm can call into
//! the adapter uniformly.

use serde::{{Deserialize, Serialize}};

use crate::build_spec::BuildSpec;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "rule", rename_all = "kebab-case")]
pub enum Violation {{
    StaleSchemaVersion {{ found: u32, expected: u32 }},
    // TODO: ecosystem-specific violations.
}}

pub fn check(spec: &BuildSpec) -> Vec<Violation> {{
    let mut out = Vec::new();
    if spec.version < crate::build_spec::SCHEMA_VERSION {{
        out.push(Violation::StaleSchemaVersion {{
            found: spec.version,
            expected: crate::build_spec::SCHEMA_VERSION,
        }});
    }}
    out
}}

pub struct {pascal}Invariants;

impl gen_types::Invariants for {pascal}Invariants {{
    type Spec = BuildSpec;
    type Violation = Violation;
    fn check(spec: &Self::Spec) -> Vec<Self::Violation> {{
        check(spec)
    }}
}}
"#,
    );
    fs::write(crate_dir.join("src/invariants.rs"), invariants_rs)?;
    let adapter_rs = format!(
        r#"//! `{pascal}Adapter` — gen-{name}'s implementation of the canonical
//! `gen_types::Adapter` trait. Stub-level today; every verb
//! returns `Unsupported` until the {name}-side parser lands.

use std::path::PathBuf;

use gen_types::{{
    Adapter, AdapterCtx, AdapterError, AdapterResult, ConfirmReport, DiffRef, DiffReport,
    LockOutcome, Plan, PlanIntent, Sbom, SbomFormat,
}};

pub struct {pascal}Adapter;

impl Adapter for {pascal}Adapter {{
    fn name(&self) -> &'static str {{ "{name}" }}
    fn manifest_files(&self) -> &'static [&'static str] {{ &["{manifest}"] }}

    fn lock(&self, _ctx: &AdapterCtx) -> AdapterResult<LockOutcome> {{
        Err(AdapterError::Unsupported("{name} lock not implemented".into()))
    }}

    fn build(&self, _ctx: &AdapterCtx) -> AdapterResult<gen_types::AdapterBuildSpec> {{
        Err(AdapterError::Unsupported("{name} build not implemented".into()))
    }}

    fn plan(&self, _ctx: &AdapterCtx, _intent: &PlanIntent) -> AdapterResult<Plan> {{
        Err(AdapterError::Unsupported("{name} plan not implemented".into()))
    }}

    fn confirm(&self, _ctx: &AdapterCtx) -> AdapterResult<ConfirmReport> {{
        Err(AdapterError::Unsupported("{name} confirm not implemented".into()))
    }}

    fn diff(&self, _ctx: &AdapterCtx, _against: &DiffRef) -> AdapterResult<DiffReport> {{
        Err(AdapterError::Unsupported("{name} diff not implemented".into()))
    }}

    fn sbom(&self, _ctx: &AdapterCtx, _format: SbomFormat) -> AdapterResult<Sbom> {{
        Err(AdapterError::Unsupported("{name} sbom not implemented".into()))
    }}

    fn quirks_registry(&self) -> Vec<gen_types::AdapterQuirkEntry> {{
        use gen_types::QuirkRegistry;
        <crate::quirks::{pascal}Quirks as QuirkRegistry>::registry()
            .into_iter()
            .map(|(p, qs)| gen_types::AdapterQuirkEntry {{
                package: p.to_string(),
                quirks: qs.into_iter().filter_map(|q| serde_json::to_value(&q).ok()).collect(),
            }})
            .collect()
    }}
}}

pub fn ctx_for(workspace_root: PathBuf) -> AdapterCtx {{
    AdapterCtx {{ workspace_root, target: None }}
}}

// Distributed-slice registration. gen-cli discovers this adapter
// at link time via the inventory iter — no per-adapter edit to
// gen-cli when this crate ships.
inventory::submit! {{
    gen_types::AdapterRegistration {{
        make: || Box::new({pascal}Adapter),
        name: "{name}",
    }}
}}
"#,
    );
    fs::write(crate_dir.join("src/adapter.rs"), adapter_rs)?;
    let tests_dir = crate_dir.join("tests");
    fs::create_dir_all(&tests_dir)?;
    let trait_surface_test = format!(
        r#"//! Trait-surface smoke tests for the scaffolded gen-{name} adapter.
//! Auto-emitted by `gen scaffold-adapter` — the four universal trait
//! surfaces (Adapter / Spec / QuirkRegistry / Invariants) must compile
//! + behave consistently from the day the crate is scaffolded.

use gen_{snake}::adapter::{pascal}Adapter;
use gen_{snake}::invariants::{pascal}Invariants;
use gen_{snake}::quirks::{pascal}Quirks;
use gen_types::{{Adapter, Invariants, QuirkRegistry}};

#[test]
fn adapter_name_matches_ecosystem() {{
    let a = {pascal}Adapter;
    assert_eq!(a.name(), "{name}");
    assert_eq!(a.manifest_files(), &["{manifest}"]);
}}

#[test]
fn empty_quirk_registry_is_callable() {{
    let names = <{pascal}Quirks as QuirkRegistry>::registered_names();
    assert!(names.is_empty());
    let q = <{pascal}Quirks as QuirkRegistry>::for_package("anything");
    assert!(q.is_empty());
}}

#[test]
fn adapter_exposes_quirks_via_default_envelope() {{
    let a = {pascal}Adapter;
    let entries = a.quirks_registry();
    assert!(entries.is_empty());
}}

#[test]
fn invariants_run_clean_against_minimal_spec() {{
    use gen_{snake}::build_spec::BuildSpec;
    use indexmap::IndexMap;
    let spec = BuildSpec {{
        version: gen_{snake}::build_spec::SCHEMA_VERSION,
        packages: IndexMap::new(),
        root_package: String::new(),
        workspace_members: vec![],
    }};
    let violations = <{pascal}Invariants as Invariants>::check(&spec);
    assert!(violations.is_empty(), "minimal spec violated: {{violations:?}}");
}}
"#,
        snake = name.replace('-', "_"),
    );
    fs::write(tests_dir.join("trait_surface.rs"), trait_surface_test)?;
    Ok(())
}

/// Stage + commit Cargo.build-spec.json when it drifted from the
/// previously-committed copy. Idempotent: no-op when nothing
/// changed. Author is `gen-spec-bot` so the commit is distinct from
/// operator-authored work — same identity gen's CI uses.
///
/// Pairs with `gen build --commit`: the local operator workflow
/// becomes `cargo update && gen build . --commit`, matching the
/// CI shape where gen-spec-bot keeps committed-bootstrap-exception
/// specs fresh by construction.
/// Commit the regenerated spec under the DELTA-ONLY doctrine.
///
/// The sole committed artifact is the slim `Cargo.gen.lock` delta. The
/// full `Cargo.build-spec.json` is retired — untracked + gitignored —
/// because substrate's lockfile-builder reconstructs it in pure Nix from
/// `Cargo.lock` + the delta (delta > build-spec > IFD), so keeping the
/// big spec in version control is redundant operator-surface noise.
///
/// CRITICAL: build-spec is only retired when a delta actually exists. A
/// single-target `gen build` emits no delta; retiring build-spec there
/// would strand the repo on IFD, so this no-ops instead.
fn commit_spec_if_drifted(root: &std::path::Path) -> Result<(), CliError> {
    use std::process::Command;
    let git = |args: &[&str]| -> Result<std::process::Output, CliError> {
        Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .map_err(|e| CliError::Other(format!("git {}: {e}", args.join(" "))))
    };

    // No delta → do not retire build-spec (would fall back to IFD).
    if !root.join("Cargo.gen.lock").exists() {
        eprintln!(
            "gen build --commit: no Cargo.gen.lock emitted (single-target?) — \
             skipping delta-only commit"
        );
        return Ok(());
    }

    // Retire the full build-spec: untrack if tracked, ensure gitignored.
    if git(&["ls-files", "--error-unmatch", "Cargo.build-spec.json"])?
        .status
        .success()
    {
        git(&["rm", "--quiet", "--cached", "--", "Cargo.build-spec.json"])?;
    }
    ensure_gitignored(root, "Cargo.build-spec.json")?;

    // Stage the delta + the .gitignore (+ the build-spec removal).
    git(&["add", "--", "Cargo.gen.lock", ".gitignore"])?;

    // Nothing staged → idempotent no-op (delta unchanged, build-spec already
    // retired + ignored).
    if git(&["diff", "--cached", "--quiet"])?.status.success() {
        eprintln!("gen build --commit: delta unchanged; no commit");
        return Ok(());
    }

    let commit = git(&[
        "-c",
        "user.name=gen-spec-bot",
        "-c",
        "user.email=gen-spec-bot@pleme-io.invalid",
        "commit",
        "--quiet",
        "-m",
        "gen: regen Cargo.gen.lock delta — delta-only (build-spec retired)",
    ])?;
    if !commit.status.success() {
        return Err(CliError::Other(format!(
            "git commit failed: {}",
            String::from_utf8_lossy(&commit.stderr).trim()
        )));
    }
    eprintln!("gen build --commit: delta committed, build-spec retired (gen-spec-bot)");
    Ok(())
}

/// Append `entry` to `<root>/.gitignore` if not already present (one entry
/// per line, exact match). Used by the delta-only commit path to retire
/// `Cargo.build-spec.json` from version control.
fn ensure_gitignored(root: &std::path::Path, entry: &str) -> Result<(), CliError> {
    let gi = root.join(".gitignore");
    let cur = std::fs::read_to_string(&gi).unwrap_or_default();
    if cur.lines().any(|l| l.trim() == entry) {
        return Ok(());
    }
    let mut new = cur;
    if !new.is_empty() && !new.ends_with('\n') {
        new.push('\n');
    }
    new.push_str(entry);
    new.push('\n');
    std::fs::write(&gi, new).map_err(|e| CliError::Other(format!("write .gitignore: {e}")))
}

fn pascalize(s: &str) -> String {
    let mut out = String::new();
    let mut upper_next = true;
    for c in s.chars() {
        if c == '-' || c == '_' {
            upper_next = true;
        } else if upper_next {
            out.push(c.to_ascii_uppercase());
            upper_next = false;
        } else {
            out.push(c);
        }
    }
    out
}

// ── `gen kanshou` — live process introspection operator surface ──────

fn kanshou_dispatch(sub: &KanshouCmd) -> Result<(), Box<dyn std::error::Error>> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async move { kanshou_run(sub).await })
}

async fn kanshou_run(sub: &KanshouCmd) -> Result<(), Box<dyn std::error::Error>> {
    match sub {
        KanshouCmd::List { app } => {
            let mut instances = kanshou::discover(app.as_deref());
            instances.sort_by(|a, b| a.app_name.cmp(&b.app_name).then(a.pid.cmp(&b.pid)));
            #[derive(serde::Serialize)]
            struct Row<'a> {
                app: &'a str,
                pid: u32,
                socket: String,
            }
            let rows: Vec<Row> = instances
                .iter()
                .map(|i| Row {
                    app: &i.app_name,
                    pid: i.pid,
                    socket: i.socket_path.display().to_string(),
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&rows)?);
            Ok(())
        }
        KanshouCmd::Query { app, path, pid } => {
            let target = pick_target(app, *pid)?;
            let mut client = kanshou::Client::connect(&target.socket_path).await?;
            let segments: Vec<String> = path.split('.').map(str::to_string).collect();
            let result = client.query(&kanshou::Query::field(segments)).await?;
            match result {
                Ok(value) => println!("{}", serde_json::to_string_pretty(&value)?),
                Err(e) => {
                    eprintln!(
                        "kanshou: query error from {} pid={}: {}",
                        target.app_name, target.pid, e
                    );
                    std::process::exit(1);
                }
            }
            Ok(())
        }
        KanshouCmd::Schema { app, pid } => {
            // Best-effort: query each top-level path on the consumer.
            // Today's Introspect trait exposes `schema()` server-side
            // but not as a wire-level query — phase 1+'s wire shape
            // is a plain value query. Fall back to an empty path
            // query which returns `unknown-field ""` plus a schema
            // hint we render verbatim.
            let target = pick_target(app, *pid)?;
            let mut client = kanshou::Client::connect(&target.socket_path).await?;
            let empty: Vec<String> = vec![];
            let probe = client.query(&kanshou::Query::field(empty)).await?;
            #[derive(serde::Serialize)]
            struct SchemaOut<'a> {
                app: &'a str,
                pid: u32,
                schema_probe: kanshou::QueryResult,
            }
            let out = SchemaOut {
                app: &target.app_name,
                pid: target.pid,
                schema_probe: probe,
            };
            println!("{}", serde_json::to_string_pretty(&out)?);
            Ok(())
        }
    }
}

fn pick_target(
    app: &str,
    pid: Option<u32>,
) -> Result<kanshou::DiscoveredInstance, Box<dyn std::error::Error>> {
    let mut instances = kanshou::discover(Some(app));
    if let Some(want) = pid {
        instances.retain(|i| i.pid == want);
    }
    instances.sort_by_key(|i| std::cmp::Reverse(i.pid));
    instances.into_iter().next().ok_or_else(|| {
        format!(
            "no live kanshou consumer for app={app}{}; check `gen kanshou list`",
            pid.map(|p| format!(" pid={p}")).unwrap_or_default()
        )
        .into()
    })
}
