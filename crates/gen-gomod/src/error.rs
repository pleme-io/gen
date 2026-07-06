use thiserror::Error;

#[derive(Debug, Error)]
pub enum GomodError {
    #[error("manifest not found: {0}")]
    ManifestNotFound(std::path::PathBuf),
    /// A triplet-interpreter phase failed. `phase` names the exact
    /// step (`go-list`, `parse-go-list`, `read-source`, `resolve-imports`,
    /// …) so consumers see the gap mechanically — never a silent wrong
    /// answer (TYPED-SPEC + INTERPRETER TRIPLET discipline).
    #[error("interp phase `{phase}`: {detail}")]
    Interp { phase: &'static str, detail: String },
    /// The `go list` subprocess itself failed (non-zero exit / spawn
    /// error). Distinct from a parse failure so the operator can tell a
    /// missing-`go` / bad-vendor-tree from malformed JSON.
    #[error("`go list` failed: {0}")]
    GoList(String),
    /// `go list -json` output did not parse as a concatenated stream of
    /// package objects.
    #[error("parsing `go list -json` output: {0}")]
    GoListParse(String),
    /// A source file named by `go list` could not be read for hashing.
    #[error("read {path}: {source}")]
    Io {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("other: {0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, GomodError>;
