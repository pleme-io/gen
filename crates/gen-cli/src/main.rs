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

#[derive(Parser, Debug)]
#[command(name = "gen", version, about = "universal package-manager engine — operator CLI")]
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
    Lock {
        path: Option<PathBuf>,
    },
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
    /// Render the workspace at <path> to Nix source (crate2nix shape).
    /// Output goes to `cfg.render.output_path` or stdout.
    Render {
        path: Option<PathBuf>,
    },
    /// Generate Cargo.features.json sidecar (cargo-metadata-driven).
    /// Substrate's lockfile-builder reads this at eval to activate the
    /// right per-crate features. Tiny (~10-20% of Cargo.nix size).
    #[command(name = "lock-features")]
    LockFeatures {
        path: Option<PathBuf>,
    },
    /// Generate the complete typed Cargo.build-spec.json.
    /// Composes Cargo.toml + Cargo.lock + cargo metadata into ONE
    /// typed JSON that substrate's lockfile-builder consumes directly.
    /// Replaces both Cargo.nix AND Cargo.features.json.
    #[command(name = "lock-build")]
    LockBuild {
        path: Option<PathBuf>,
    },
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
    /// Probe the configured substituters for every lockfile entry in
    /// the workspace at <path>. Report hit / miss / hit-rate.
    #[command(name = "cache-probe")]
    CacheProbe {
        path: Option<PathBuf>,
    },
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
            Self::Custom => ConfigTier::Custom(
                custom_path.unwrap_or_else(|| PathBuf::from(".gen.yaml")),
            ),
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
        Cmd::Lock { path } => {
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
            let manifest = dispatch(&root, cfg)?;
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
        Cmd::Build { path, filter_platform, single_target } => {
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
                    let out = if let Some(triple) = filter_platform {
                        gen_cargo::build_spec::generate_for_target_and_write(&root, &triple)
                            .map_err(CliError::Cargo)?
                    } else if *single_target {
                        gen_cargo::build_spec::generate_and_write(&root)
                            .map_err(CliError::Cargo)?
                    } else {
                        gen_cargo::build_spec::generate_multi_target_and_write(&root)
                            .map_err(CliError::Cargo)?
                    };
                    eprintln!("gen build: wrote {}", out.display());
                }
                other => {
                    return Err(CliError::RenderNotImplementedForAdapter(other.to_string()));
                }
            }
            Ok(())
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
        Cmd::Scaffold { shape, name, dir, owner } => {
            scaffold_repo(*shape, name, dir.as_deref(), owner)
                .map_err(|e| CliError::Other(format!("scaffold: {e}")))
        }
        Cmd::ScaffoldAdapter { name, manifest, dir } => {
            scaffold_adapter(name, manifest, dir.as_deref())
                .map_err(|e| CliError::Other(format!("scaffold-adapter: {e}")))?;
            println!(
                "✓ gen-{name} scaffolded.\n  Next: add `\"crates/gen-{name}\",` to the gen workspace members in `Cargo.toml`, fill in adapter::Adapter::build, then `cargo build -p gen-{name}`."
            );
            Ok(())
        }
        Cmd::Quirks { adapter } => {
            // Operator-visible introspection of every adapter's typed
            // quirk registry. The CargoAdapter exposes the CrateQuirk
            // registry via the Adapter::quirks_registry surface; future
            // npm + bundler adapters expose their own typed shapes via
            // the same trait method. One verb, every ecosystem.
            #[derive(serde::Serialize)]
            struct AdapterQuirks<'a> {
                adapter: &'a str,
                entries: Vec<gen_types::AdapterQuirkEntry>,
            }
            let mut out: Vec<AdapterQuirks> = Vec::new();
            // Cargo
            let cargo = gen_cargo::adapter::CargoAdapter;
            if adapter.as_deref().map(|a| a == "cargo").unwrap_or(true) {
                out.push(AdapterQuirks {
                    adapter: "cargo",
                    entries: gen_types::Adapter::quirks_registry(&cargo),
                });
            }
            // npm + bundler: stubs return empty Vec via default impl.
            // Surface them so operators see what's available.
            if adapter.as_deref().map(|a| a == "npm").unwrap_or(true) {
                let npm = gen_npm::adapter::NpmAdapter;
                out.push(AdapterQuirks {
                    adapter: "npm",
                    entries: gen_types::Adapter::quirks_registry(&npm),
                });
            }
            if adapter.as_deref().map(|a| a == "bundler").unwrap_or(true) {
                let bundler = gen_bundler::adapter::BundlerAdapter;
                out.push(AdapterQuirks {
                    adapter: "bundler",
                    entries: gen_types::Adapter::quirks_registry(&bundler),
                });
            }
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
        target.join(".github").join("workflows").join("auto-release.yml"),
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

/// Adapter dispatch — selects the matching parser based on the marker
/// file present at `root`. Honors `cfg.workspace.force_adapter` when
/// set, otherwise probes the routing table in declaration order.
fn dispatch(
    root: &std::path::Path,
    cfg: &GenConfig,
) -> Result<gen_types::Manifest, CliError> {
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

fn pick_adapter(
    root: &std::path::Path,
    cfg: &GenConfig,
) -> Result<String, CliError> {
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
fn scaffold_adapter(
    name: &str,
    manifest: &str,
    dir: Option<&Path>,
) -> Result<(), std::io::Error> {
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
"#,
    );
    fs::write(crate_dir.join("src/adapter.rs"), adapter_rs)?;
    Ok(())
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
