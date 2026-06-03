use thiserror::Error;

#[derive(Debug, Error)]
pub enum GomodError {
    #[error("manifest not found: {0}")]
    ManifestNotFound(std::path::PathBuf),
    #[error(
        "failed to prefetch vendor hash for module at {root}: {reason}. \
The build spec MUST carry the exact `vendorHash` nixpkgs `buildGoModule` \
expects; a wrong (or missing) hash produces a non-FOD fetch derivation \
that is denied network access in the substrate sandbox and explodes \
downstream with a hash mismatch during the fleet build. The prefetcher \
runs one `go` subprocess (go mod download/vendor with checksum \
verification on) then NAR-sha256s the vendor tree in-process — check \
`go` is on PATH, the module's go.sum is complete, and the proxy/network \
is reachable, then re-run `gen build`."
    )]
    PrefetchVendorHashFailed {
        root: std::path::PathBuf,
        reason: String,
    },
    #[error("other: {0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, GomodError>;
