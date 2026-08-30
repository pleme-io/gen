//! Test support — a deterministic, filesystem-free [`GoBuildEnv`].
//!
//! The Environment trait ([`crate::interp::GoBuildEnv`]) IS the
//! testability contract: [`MockGoBuildEnv`] satisfies it from in-memory
//! maps so the entire encoder runs with ZERO `go` / filesystem
//! dependency (integration tests + downstream consumers alike). Shipped
//! in the crate (not `#[cfg(test)]`) exactly because integration-test
//! crates and downstream verifiers need it — mirrors gen-cargo shipping
//! its `MockResolver`.

use std::collections::HashMap;
use std::path::Path;

use crate::build_spec::TargetTuple;
use crate::error::GomodError;
use crate::interp::GoBuildEnv;

/// A [`GoBuildEnv`] backed by in-memory maps.
#[derive(Clone, Debug, Default)]
pub struct MockGoBuildEnv {
    /// Canned `go list` JSON keyed by [`TargetTuple::suffix`], so a
    /// multi-tuple mock (linux vs darwin build-tag differences) works.
    pub list_by_tuple: HashMap<String, String>,
    /// Canned file contents keyed by absolute path string (go.mod,
    /// go.sum, each source/embed file).
    pub files: HashMap<String, Vec<u8>>,
}

impl MockGoBuildEnv {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register the `go list` output for a tuple (builder style).
    #[must_use]
    pub fn with_list(mut self, tuple: &TargetTuple, json: impl Into<String>) -> Self {
        self.list_by_tuple.insert(tuple.suffix(), json.into());
        self
    }

    /// Register a file's bytes at an absolute path (builder style).
    #[must_use]
    pub fn with_file(mut self, path: impl Into<String>, bytes: impl Into<Vec<u8>>) -> Self {
        self.files.insert(path.into(), bytes.into());
        self
    }
}

impl GoBuildEnv for MockGoBuildEnv {
    fn go_list(&self, _root: &Path, tuple: &TargetTuple) -> Result<String, GomodError> {
        self.list_by_tuple
            .get(&tuple.suffix())
            .cloned()
            .ok_or_else(|| {
                GomodError::GoList(format!(
                    "mock: no go list registered for {}",
                    tuple.suffix()
                ))
            })
    }

    fn read_file(&self, path: &Path) -> Result<Vec<u8>, GomodError> {
        let key = path.to_string_lossy().to_string();
        self.files.get(&key).cloned().ok_or_else(|| GomodError::Io {
            path: path.to_path_buf(),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "mock: no file registered"),
        })
    }
}
