//! `Go.build-spec.json` emission — the operator-facing write surface.
//!
//! Mirrors gen-cargo's `build_spec::generate_and_write`: the encoder
//! ([`crate::interp::apply`]) produces a typed [`BuildSpec`]; this module
//! serializes it to canonical pretty JSON and writes it to
//! `<root>/Go.build-spec.json`. `gen build .` on a Go module routes here.
//!
//! Every side effect stays behind the [`GoBuildEnv`] seam
//! (TYPED-SPEC + INTERPRETER TRIPLET): the mockable core
//! [`generate_with_env`] takes a `&dyn GoBuildEnv`, so the whole
//! emit path — including the file write — is exercised filesystem-free
//! in tests via [`crate::testkit::MockGoBuildEnv`]. The public
//! [`generate_and_write`] / [`generate_for_target_and_write`] wrappers
//! construct the real [`RealGoBuildEnv`] for the operator path.

use std::path::{Path, PathBuf};

use crate::build_spec::{BuildSpec, TargetTuple};
use crate::error::GomodError;
use crate::interp::{EncodeCtx, GoBuildEnv, RealGoBuildEnv, apply};

/// Canonical filename the substrate `package-builder.nix` interpreter
/// reads. The gomod peer of gen-cargo's `Cargo.build-spec.json`.
pub const SPEC_FILENAME: &str = "Go.build-spec.json";

/// Serialize a [`BuildSpec`] to canonical pretty JSON (trailing
/// newline). Deterministic by construction — the spec's keyed maps are
/// `BTreeMap`s, so the byte output is stable regardless of encode order.
pub fn to_json(spec: &BuildSpec) -> Result<String, GomodError> {
    let mut body = serde_json::to_string_pretty(spec)
        .map_err(|e| GomodError::Other(format!("serialize Go.build-spec.json: {e}")))?;
    body.push('\n');
    Ok(body)
}

/// Mockable core: encode the module at `root` for `tuple` via the given
/// [`GoBuildEnv`], write `Go.build-spec.json` next to `go.mod`, and
/// return the written path. Tests drive this with a `MockGoBuildEnv`;
/// the real path calls it through [`generate_for_target_and_write`].
pub fn generate_with_env(
    env: &dyn GoBuildEnv,
    root: &Path,
    tuple: TargetTuple,
) -> Result<PathBuf, GomodError> {
    let spec = apply(
        env,
        &EncodeCtx {
            root: root.to_path_buf(),
            tuple,
        },
    )?;
    let body = to_json(&spec)?;
    let out = root.join(SPEC_FILENAME);
    std::fs::write(&out, body).map_err(|source| GomodError::Io {
        path: out.clone(),
        source,
    })?;
    Ok(out)
}

/// Emit `Go.build-spec.json` for the module at `root` targeting the
/// build host's tuple (M1 default). The operator entrypoint reached by
/// `gen build .` on a Go module.
pub fn generate_and_write(root: &Path) -> Result<PathBuf, GomodError> {
    generate_with_env(&RealGoBuildEnv::default(), root, TargetTuple::host())
}

/// Per-target variant of [`generate_and_write`]. `triple` is a Rust
/// target triple (e.g. `x86_64-unknown-linux-musl`); it is mapped to a
/// Go `(goos, goarch)` tuple. Unrecognized triples fall back to the
/// host tuple rather than fabricating a bad target — the encoder then
/// emits for the host, and the caller sees the host suffix in the spec.
pub fn generate_for_target_and_write(root: &Path, triple: &str) -> Result<PathBuf, GomodError> {
    let tuple = TargetTuple::from_rust_triple(triple).unwrap_or_else(TargetTuple::host);
    generate_with_env(&RealGoBuildEnv::default(), root, tuple)
}
