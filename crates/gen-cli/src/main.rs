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
            let manifest = gen_cargo::parse(&root).map_err(CliError::Cargo)?;
            emit(&manifest, cli.format)
        }
        Cmd::Lock { path } => {
            let root = resolve_root(path, cfg);
            let manifest = gen_cargo::parse(&root).map_err(CliError::Cargo)?;
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
    }
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
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Yaml(#[from] serde_yaml::Error),
}

fn _resolve_root_lint_silencer(p: &Path) -> PathBuf {
    p.to_path_buf()
}
