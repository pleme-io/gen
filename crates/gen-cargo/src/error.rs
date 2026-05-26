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
}

pub type Result<T> = std::result::Result<T, CargoError>;
