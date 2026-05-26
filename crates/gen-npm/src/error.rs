//! Typed errors for the npm adapter.

use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum NpmError {
    #[error("failed to read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse {path} as JSON: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to parse {path} as YAML: {source}")]
    Yaml {
        path: PathBuf,
        #[source]
        source: serde_yaml::Error,
    },
    #[error("workspace root at {root} declared member {member} that does not exist")]
    MissingWorkspaceMember { root: PathBuf, member: String },
    #[error("version `{raw}` for {context} could not be parsed")]
    BadVersion { raw: String, context: String },
    #[error("dependency `{name}` requirement `{raw}` could not be parsed")]
    BadVersionReq { name: String, raw: String },
}

pub type Result<T> = std::result::Result<T, NpmError>;
