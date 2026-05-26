//! Typed errors for the Bundler adapter.

use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum BundlerError {
    #[error("failed to read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("dependency `{name}` requirement `{raw}` could not be parsed")]
    BadVersionReq { name: String, raw: String },
}

pub type Result<T> = std::result::Result<T, BundlerError>;
