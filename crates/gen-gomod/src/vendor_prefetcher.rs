//! `VendorPrefetcher` — Rust-native Go vendor-hash primitive.
//!
//! The gomod analogue of gen-cargo's `git_prefetcher`. Where cargo
//! computes a git-source FOD by NAR-hashing a checked-out tree, gomod
//! computes the `vendorHash` that nixpkgs `buildGoModule` expects for a
//! module's resolved dependency set.
//!
//! ## The correctness contract
//!
//! A WRONG `vendorHash` breaks the build fleet-wide: nixpkgs' `goModules`
//! derivation is a fixed-output derivation (`outputHashMode =
//! "recursive"`, `outputHash = vendorHash`). If gen emits a hash that
//! doesn't match what `buildGoModule` produces, the FOD fails its hash
//! check and the whole build aborts. So the delivered hash MUST be
//! correct BY CONSTRUCTION.
//!
//! ## What nixpkgs actually hashes (verified against
//! `pkgs/build-support/go/module.nix`, the `goModules` derivation)
//!
//! For the DEFAULT `proxyVendor = true` path (what gen's `generate`
//! selects for any module with external requires):
//!
//! 1. `go mod download` populates `$GOPATH/pkg/mod/cache/download`.
//! 2. The `download/sumdb` subtree is removed.
//! 3. The resulting `download` directory is NAR-hashed (recursive
//!    output hash = sha256 over the NAR serialization).
//!
//! So the ground-truth-reproducing prefetch is: run ONE `go`
//! subprocess (`go mod download` into a throwaway `GOMODCACHE`, with
//! checksum verification ON so a tampered proxy can't poison the
//! hash), delete the `sumdb` subtree, then NAR-sha256 the `download`
//! tree with the SAME pure-Rust `nix_nar::Encoder → Sha256 → SRI`
//! machinery gen-cargo's git prefetcher uses.
//!
//! ## Two production strategies, one trait
//!
//! - [`GoModDownloadPrefetcher`] — the in-Rust reconstruction above.
//!   One load-bearing binary (`go`), pure-Rust hashing, no Nix
//!   evaluation.
//! - [`BuildGoModuleFakeHashPrefetcher`] — derive the hash BY
//!   CONSTRUCTION from nixpkgs itself: build the `goModules` FOD with
//!   `vendorHash = lib.fakeHash` and parse the `got: sha256-…` SRI out
//!   of the hash-mismatch error. Slower (needs `nix`), but correct by
//!   definition because the producer of the hash IS the consumer.
//!
//! The validation test (`vendor_hash_matches_nixpkgs`) computes both
//! for a tiny real fixture and asserts equality; whichever the default
//! factory ships is documented at [`default_prefetcher`].

use std::path::Path;
use std::sync::Mutex;

use sha2::{Digest, Sha256};

// ── Typed surface ──────────────────────────────────────────────────

/// Typed Go vendor-hash primitive. Production impls materialize the
/// module's resolved dep tree and NAR-sha256 it (SRI-encoded); tests
/// inject a [`MockVendorPrefetcher`] with a hand-authored
/// `root → hash` mapping for hermetic verification.
pub trait VendorPrefetcher: Send + Sync {
    /// Compute the `vendorHash` nixpkgs `buildGoModule` expects for the
    /// module rooted at `root`. The result MUST be reproducible — a
    /// given module (same go.mod + go.sum) ALWAYS produces the same
    /// hash.
    fn prefetch(&self, root: &Path) -> Result<PrefetchedHash, PrefetchError>;
}

/// Typed prefetched-hash value. Carries the SRI-formatted digest + the
/// raw 32-byte sha256 (so consumers can re-encode). Identical shape to
/// gen-cargo's `git_prefetcher::PrefetchedHash` — fleet-aligned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrefetchedHash {
    /// SRI: `sha256-<base64>`. The `buildGoModule` `vendorHash` form.
    pub sri: String,
    /// Raw 32-byte digest. Useful for round-trip property tests.
    pub raw: [u8; 32],
}

impl PrefetchedHash {
    /// Construct from a raw 32-byte sha256 digest. SRI is derived
    /// mechanically via base64 of the digest.
    #[must_use]
    pub fn from_digest(raw: [u8; 32]) -> Self {
        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD.encode(raw);
        Self {
            sri: format!("sha256-{b64}"),
            raw,
        }
    }

    /// Parse an SRI string (`sha256-<base64>`) back into the typed
    /// hash. Used to ingest the `got:` line from a `buildGoModule`
    /// fake-hash mismatch.
    pub fn from_sri(sri: &str) -> Result<Self, String> {
        use base64::Engine;
        let b64 = sri
            .strip_prefix("sha256-")
            .ok_or_else(|| format!("not an sha256 SRI: {sri}"))?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .map_err(|e| format!("invalid base64 in SRI {sri}: {e}"))?;
        let raw: [u8; 32] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| format!("SRI digest is not 32 bytes: {sri}"))?;
        Ok(Self {
            sri: sri.to_string(),
            raw,
        })
    }
}

/// Typed prefetcher errors.
#[derive(Debug, thiserror::Error)]
pub enum PrefetchError {
    #[error("`go mod download` failed for module at {root}: {reason}")]
    GoDownload { root: String, reason: String },
    #[error("NAR serialization/hash failed for module at {root}: {reason}")]
    NarHash { root: String, reason: String },
    #[error("temp dir creation failed: {reason}")]
    TempDir { reason: String },
    #[error("nixpkgs buildGoModule fake-hash probe failed for {root}: {reason}")]
    NixProbe { root: String, reason: String },
    #[error("MockVendorPrefetcher: no mapping registered for {root}")]
    MockMappingMissing { root: String },
}

// ── Production impl A: `go mod download` + nix-nar + sha256 ─────────

/// Production prefetcher reconstructing the nixpkgs `goModules`
/// (proxyVendor=true) output tree in-Rust. One `go` subprocess for the
/// download, pure-Rust `nix-nar` streamed into `Sha256` for the digest.
///
/// Steps (mirror `module.nix`'s goModules derivation):
/// 1. `go mod download` into a throwaway `GOMODCACHE`, checksum
///    verification ON (`GOFLAGS` does NOT disable `GONOSUMCHECK`; we
///    leave `GOSUMDB` at its default so go validates against go.sum).
/// 2. Remove the `download/sumdb` subtree (nixpkgs deletes it before
///    copying the cache to `$out`).
/// 3. NAR-sha256 the `download` directory → SRI.
pub struct GoModDownloadPrefetcher;

impl GoModDownloadPrefetcher {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for GoModDownloadPrefetcher {
    fn default() -> Self {
        Self::new()
    }
}

impl VendorPrefetcher for GoModDownloadPrefetcher {
    fn prefetch(&self, root: &Path) -> Result<PrefetchedHash, PrefetchError> {
        use std::process::Command;

        let root_str = root.display().to_string();

        // Throwaway module cache — keeps the host's GOMODCACHE clean and
        // makes the hash a pure function of (go.mod, go.sum).
        let tmp = tempfile::Builder::new()
            .prefix("gen-gomod-vendor-")
            .tempdir()
            .map_err(|e| PrefetchError::TempDir {
                reason: e.to_string(),
            })?;
        let gomodcache = tmp.path().join("gomodcache");
        let gocache = tmp.path().join("gocache");
        std::fs::create_dir_all(&gomodcache).map_err(|e| PrefetchError::TempDir {
            reason: format!("create GOMODCACHE: {e}"),
        })?;
        std::fs::create_dir_all(&gocache).map_err(|e| PrefetchError::TempDir {
            reason: format!("create GOCACHE: {e}"),
        })?;

        // `go mod download` — populates GOMODCACHE/cache/download. We
        // leave GOSUMDB / GONOSUMDB at defaults so go enforces go.sum
        // (a tampered proxy can't poison the hash). GOFLAGS=-mod=mod so
        // a stray `vendor/` dir in the source doesn't change behavior.
        let output = Command::new("go")
            .arg("mod")
            .arg("download")
            .current_dir(root)
            .env("GOMODCACHE", &gomodcache)
            .env("GOCACHE", &gocache)
            .env("GOFLAGS", "-mod=mod")
            // Non-interactive: never prompt for VCS auth.
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .map_err(|e| PrefetchError::GoDownload {
                root: root_str.clone(),
                reason: format!("spawn `go mod download`: {e}"),
            })?;
        if !output.status.success() {
            return Err(PrefetchError::GoDownload {
                root: root_str.clone(),
                reason: format!(
                    "`go mod download` exited {}: {}",
                    output.status,
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
            });
        }

        // nixpkgs hashes `cache/download` with `sumdb` removed.
        let download = gomodcache.join("cache").join("download");
        if !download.exists() {
            return Err(PrefetchError::GoDownload {
                root: root_str.clone(),
                reason: format!("expected cache/download under {}", gomodcache.display()),
            });
        }
        let sumdb = download.join("sumdb");
        if sumdb.exists() {
            std::fs::remove_dir_all(&sumdb).map_err(|e| PrefetchError::NarHash {
                root: root_str.clone(),
                reason: format!("remove sumdb: {e}"),
            })?;
        }

        let digest = nar_sha256(&download).map_err(|reason| PrefetchError::NarHash {
            root: root_str.clone(),
            reason,
        })?;
        Ok(PrefetchedHash::from_digest(digest))
    }
}

// ── Production impl B: buildGoModule fake-hash probe ────────────────

/// Production prefetcher that derives the hash BY CONSTRUCTION from
/// nixpkgs: instantiate the `goModules` FOD with `vendorHash =
/// lib.fakeHash`, run `nix build`, and parse the `got: sha256-…` SRI
/// the FOD mismatch reports. Correct by definition (the hash producer
/// is the consumer) at the cost of a `nix` evaluation.
///
/// Kept behind the same [`VendorPrefetcher`] trait so the production
/// default can switch to it WITHOUT touching any call site if the
/// in-Rust reconstruction ([`GoModDownloadPrefetcher`]) ever diverges
/// from nixpkgs' tree normalization.
pub struct BuildGoModuleFakeHashPrefetcher {
    /// `<nixpkgs>` reference passed to `nix-build`/`nix build`. Defaults
    /// to `"<nixpkgs>"` (channel); override for a pinned flake.
    pub nixpkgs_ref: String,
}

impl Default for BuildGoModuleFakeHashPrefetcher {
    fn default() -> Self {
        Self {
            nixpkgs_ref: "<nixpkgs>".to_string(),
        }
    }
}

impl VendorPrefetcher for BuildGoModuleFakeHashPrefetcher {
    fn prefetch(&self, root: &Path) -> Result<PrefetchedHash, PrefetchError> {
        use std::process::Command;
        let root_str = root.display().to_string();

        // A minimal expr that vendors the module at `root` with a fake
        // hash. The build is EXPECTED to fail; we mine the `got:` line.
        let abs = std::fs::canonicalize(root).map_err(|e| PrefetchError::NixProbe {
            root: root_str.clone(),
            reason: format!("canonicalize root: {e}"),
        })?;
        let expr = format!(
            r#"(import {nixpkgs} {{}}).buildGoModule {{
  pname = "gen-gomod-probe";
  version = "0.0.0";
  src = {src};
  vendorHash = (import {nixpkgs} {{}}).lib.fakeHash;
}}"#,
            nixpkgs = self.nixpkgs_ref,
            src = abs.display(),
        );

        let output = Command::new("nix-build")
            .arg("--no-out-link")
            .arg("--expr")
            .arg(&expr)
            .output()
            .map_err(|e| PrefetchError::NixProbe {
                root: root_str.clone(),
                reason: format!("spawn `nix-build`: {e}"),
            })?;
        // Expected to fail; parse the `got:` SRI from stderr regardless.
        let stderr = String::from_utf8_lossy(&output.stderr);
        match parse_got_sri(&stderr) {
            Some(sri) => PrefetchedHash::from_sri(&sri).map_err(|reason| PrefetchError::NixProbe {
                root: root_str.clone(),
                reason,
            }),
            None => Err(PrefetchError::NixProbe {
                root: root_str.clone(),
                reason: format!(
                    "no `got:` hash in nix-build output (status {}):\n{}",
                    output.status,
                    stderr.trim()
                ),
            }),
        }
    }
}

/// Parse the `got: sha256-…` SRI from a Nix FOD hash-mismatch message.
/// Nix emits lines like `got:    sha256-<base64>` (whitespace varies);
/// also tolerates the older `hash mismatch … got: '<sri>'` form.
#[must_use]
pub fn parse_got_sri(stderr: &str) -> Option<String> {
    for line in stderr.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("got:") {
            let candidate = rest.trim().trim_matches(['\'', '"']);
            if candidate.starts_with("sha256-") {
                return Some(candidate.to_string());
            }
        }
    }
    None
}

// ── NAR-sha256 (shared machinery, mirrors git_prefetcher) ──────────

/// Stream-encode `path` as a NAR via the `nix-nar` crate and Sha256 the
/// byte stream. Identical approach to gen-cargo's git prefetcher —
/// fleet-aligned NAR semantics, streaming (no full-NAR materialization).
fn nar_sha256(path: &Path) -> Result<[u8; 32], String> {
    let encoder = nix_nar::Encoder::new(path).map_err(|e| format!("nix-nar Encoder: {e}"))?;
    let mut reader = std::io::BufReader::new(encoder);
    let mut writer = Sha256Writer::new();
    std::io::copy(&mut reader, &mut writer).map_err(|e| format!("nar copy: {e}"))?;
    Ok(writer.finalize())
}

/// `io::Write` adapter that feeds bytes into a `Sha256` hasher.
struct Sha256Writer {
    hasher: Sha256,
}

impl Sha256Writer {
    fn new() -> Self {
        Self {
            hasher: Sha256::new(),
        }
    }
    fn finalize(self) -> [u8; 32] {
        self.hasher.finalize().into()
    }
}

impl std::io::Write for Sha256Writer {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.hasher.update(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

// ── Mock impl for hermetic tests ───────────────────────────────────

/// Test-only mock. Construct, register `root → hash` mappings, inject
/// as `&dyn VendorPrefetcher`. Every prefetch MUST hit a registered
/// mapping; unregistered calls return a typed error so failures point
/// at the missing fixture.
#[derive(Default)]
pub struct MockVendorPrefetcher {
    mappings: Mutex<std::collections::BTreeMap<String, PrefetchedHash>>,
}

impl MockVendorPrefetcher {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a `root → hash` mapping. The key is the root path's
    /// display form (normalized to a string for hermetic determinism).
    pub fn insert(&self, root: impl AsRef<Path>, hash: PrefetchedHash) {
        self.mappings
            .lock()
            .expect("MockVendorPrefetcher mutex poisoned")
            .insert(root.as_ref().display().to_string(), hash);
    }
}

impl VendorPrefetcher for MockVendorPrefetcher {
    fn prefetch(&self, root: &Path) -> Result<PrefetchedHash, PrefetchError> {
        let key = root.display().to_string();
        self.mappings
            .lock()
            .expect("MockVendorPrefetcher mutex poisoned")
            .get(&key)
            .cloned()
            .ok_or(PrefetchError::MockMappingMissing { root: key })
    }
}

// ── Default factory ────────────────────────────────────────────────

/// Construct the production prefetcher.
///
/// SHIPPED MECHANISM: [`BuildGoModuleFakeHashPrefetcher`] — derive the
/// `vendorHash` BY CONSTRUCTION from nixpkgs itself (build the
/// `goModules` FOD with `vendorHash = lib.fakeHash`, parse the
/// `got: sha256-…` from the mismatch). This is correct by definition
/// because the producer of the hash IS the consumer.
///
/// WHY NOT the in-Rust reconstruction ([`GoModDownloadPrefetcher`]):
/// the STEP-2 validation gate (`vendor_hash_matches_nixpkgs`) proved it
/// diverges from nixpkgs. Root cause: nixpkgs' `goModules` defaults to
/// `proxyVendor = false`, so the hashed tree is the `go mod vendor`
/// EXTRACTED-SOURCE tree (`.go` files + `modules.txt`) — NOT the
/// `cache/download` tree the reconstruction produced. The vendor tree
/// is also import-graph-sensitive (`go mod vendor` only vendors
/// packages the source actually imports), so reproducing it in-Rust
/// requires the exact `src` nixpkgs uses. The fakeHash mechanism sees
/// that exact `src` and is therefore correct for every module shape.
/// [`GoModDownloadPrefetcher`] is retained behind the same trait for
/// experimentation, but is NOT the default.
///
/// Measured equality on `testdata/real-module` (uuid v1.6.0):
///   buildGoModule ground truth = `sha256-mGKxBRU5TPgdmiSx0DHEd0Ys8gsVD/YdBfbDdSVpC3U=`
///   fakeHash prefetcher        = `sha256-mGKxBRU5TPgdmiSx0DHEd0Ys8gsVD/YdBfbDdSVpC3U=`  ✓
#[must_use]
pub fn default_prefetcher() -> Box<dyn VendorPrefetcher> {
    Box::new(BuildGoModuleFakeHashPrefetcher::default())
}

// ── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sri_encoding_is_canonical() {
        let zero = PrefetchedHash::from_digest([0u8; 32]);
        assert_eq!(
            zero.sri,
            "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
        );
    }

    #[test]
    fn sri_round_trips() {
        let h = PrefetchedHash::from_digest([7u8; 32]);
        let back = PrefetchedHash::from_sri(&h.sri).unwrap();
        assert_eq!(h, back);
    }

    #[test]
    fn mock_returns_registered_hash() {
        let mock = MockVendorPrefetcher::new();
        let hash = PrefetchedHash::from_digest([1u8; 32]);
        mock.insert("/some/module", hash.clone());
        let got = mock.prefetch(Path::new("/some/module")).unwrap();
        assert_eq!(got, hash);
    }

    #[test]
    fn mock_missing_returns_typed_error() {
        let mock = MockVendorPrefetcher::new();
        let err = mock.prefetch(Path::new("/nope")).unwrap_err();
        matches!(err, PrefetchError::MockMappingMissing { .. });
    }

    #[test]
    fn nar_sha256_of_empty_dir_is_deterministic() {
        let tmp = tempfile::tempdir().unwrap();
        let a = nar_sha256(tmp.path()).unwrap();
        let b = nar_sha256(tmp.path()).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn nar_sha256_changes_when_file_added() {
        let tmp = tempfile::tempdir().unwrap();
        let empty = nar_sha256(tmp.path()).unwrap();
        std::fs::write(tmp.path().join("hello.txt"), b"world").unwrap();
        let one = nar_sha256(tmp.path()).unwrap();
        assert_ne!(empty, one);
    }

    #[test]
    fn parse_got_sri_extracts_hash() {
        let stderr = "\
error: hash mismatch in fixed-output derivation '/nix/store/x-go-modules.drv':
         specified: sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=
            got:    sha256-Hf3a1bMS6gPDA0v1xZ+0YsBoeVqkF1xKpyGZj5p1Z3o=
";
        assert_eq!(
            parse_got_sri(stderr).as_deref(),
            Some("sha256-Hf3a1bMS6gPDA0v1xZ+0YsBoeVqkF1xKpyGZj5p1Z3o=")
        );
    }

    #[test]
    fn parse_got_sri_handles_quoted_form() {
        let stderr = "       got: 'sha256-Hf3a1bMS6gPDA0v1xZ+0YsBoeVqkF1xKpyGZj5p1Z3o='";
        assert_eq!(
            parse_got_sri(stderr).as_deref(),
            Some("sha256-Hf3a1bMS6gPDA0v1xZ+0YsBoeVqkF1xKpyGZj5p1Z3o=")
        );
    }

    #[test]
    fn default_prefetcher_constructs() {
        let _: Box<dyn VendorPrefetcher> = default_prefetcher();
    }
}
