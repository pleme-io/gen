//! Typed errors emitted by the cargo adapter. Every parse failure
//! goes through one variant — operators get a structured cause +
//! filename + line where possible, never a stringly-typed message.

use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CargoError {
    #[error("failed to read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse {path} as TOML: {source}")]
    Toml {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("workspace root at {root} declared member {member} but the path does not exist")]
    MissingWorkspaceMember { root: PathBuf, member: String },
    #[error("Cargo.toml at {path} has neither [package] nor [workspace]")]
    EmptyManifest { path: PathBuf },
    #[error("dependency `{name}` in {path} uses an unsupported source shape: {detail}")]
    UnsupportedDepSource {
        name: String,
        path: PathBuf,
        detail: String,
    },
    #[error("Cargo.lock at {path} entry {entry} is missing a required field: {field}")]
    LockfileMissingField {
        path: PathBuf,
        entry: String,
        field: &'static str,
    },
    #[error("version `{raw}` for {context} could not be parsed")]
    BadVersion { raw: String, context: String },
    #[error("dependency `{name}` requirement `{raw}` could not be parsed")]
    BadVersionReq { name: String, raw: String },
    #[error(
        "external path-dep `{name}` (version {version}) at {abs_dir} escapes the workspace root \
({workspace_root}) AND could not be resolved as a git source: {reason}. \
Substrate's nix lockfile-builder cannot consume `path = \"../…\"` deps — the sibling repo's \
source is not in the build sandbox. Either:\n\
  1. Convert the dep to a git source in Cargo.toml:\n\
       {name} = {{ git = \"https://github.com/<owner>/<repo>\" }}\n\
  2. Ensure the sibling directory is a git repo with an `origin` remote pointing at a public host \
(GitHub) so gen can auto-resolve it.\n\
Once the Cargo.toml dep is git-based, run `gen build --if-stale` to regenerate the spec."
    )]
    UnresolvableExternalPath {
        name: String,
        version: String,
        abs_dir: PathBuf,
        workspace_root: PathBuf,
        reason: String,
    },
    #[error(
        "failed to prefetch sha256 for git source {url}#{rev}: {reason}. \
The build spec MUST carry a fixed sha256 for every git source; emitting one \
without sha256 would produce a non-FOD `pkgs.fetchgit` derivation that is \
denied network access in the substrate sandbox and explodes downstream with \
cryptic 'Could not resolve host' errors during nixos-rebuild. The prefetcher \
is pure-Rust (gix + nix-nar) — check the remote is reachable (host DNS, \
auth tokens for private repos) and the rev exists, then re-run `gen build`."
    )]
    PrefetchSha256Failed {
        url: String,
        rev: String,
        reason: String,
    },
}

pub type Result<T> = std::result::Result<T, CargoError>;
