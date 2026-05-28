use thiserror::Error;

#[derive(Debug, Error)]
pub enum PipError {
    #[error("manifest not found: {0}")]
    ManifestNotFound(std::path::PathBuf),
    #[error("other: {0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, PipError>;
