//! `GitPrefetcher` — pure-Rust git prefetch primitive.
//!
//! Replaces the brittle `nix-prefetch-git` shell-script call chain
//! that gen-cargo previously shelled out to. Every operation runs
//! in-process: `gix` for the git fetch (no `git` binary), `nix-nar`
//! for streaming NAR serialization into Sha256 (no `nix` binary,
//! no `nix-prefetch-git` shell script). Aligns with sui-compat's
//! NAR semantics — `nix-nar` is the same crate sui-compat wraps for
//! its `NarWriter::write_path` operator.
//!
//! Architecture: typed trait + mock for hermetic testing + production
//! `GixPrefetcher` impl. The build-spec generator depends on the trait
//! object so unit tests can swap in `MockPrefetcher` with hand-authored
//! `(url, rev) → sha256` mappings, no network, no tempdirs, no flakiness.
//!
//! Theory: `theory/RUST-NATIVE-PREFETCH.md`.
//! Authored as `(deftyped-prefetcher gen.cargo.git-prefetcher …)`
//! in `gen-cargo/specs/prefetcher.lisp` (forthcoming).

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;

use sha2::{Digest, Sha256};

// ── Typed surface ──────────────────────────────────────────────────

/// Typed git-prefetch primitive. Production impl clones via `gix`
/// and hashes via streaming NAR-sha256 (in-Rust). Tests inject a
/// `MockPrefetcher` with hand-authored `(url, rev) → hash` mappings
/// for hermetic verification.
pub trait GitPrefetcher: Send + Sync {
    /// Fetch the tree at `rev` from `url` and return its NAR-sha256
    /// digest, SRI-encoded. The result MUST be reproducible — a
    /// given `(url, rev)` ALWAYS produces the same hash.
    fn prefetch(&self, url: &str, rev: &str) -> Result<PrefetchedHash, PrefetchError>;
}

/// Typed prefetched-hash value. Carries the SRI-formatted digest +
/// the underlying raw 32-byte sha256 (so consumers can re-encode).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrefetchedHash {
    /// SRI: `sha256-<base64>`. The fetchgit-accepted form.
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
}

/// Typed prefetcher errors. Catalog-registered via the
/// `#[gen_macros::fsm]` attribute so operator reflection (`gen
/// dispatchers catalog`) lists every prefetcher failure class —
/// future remediations (Shigoto retry policies, drift-detector
/// dispatch tables, alert-on-fetch-failure controllers) consume the
/// catalog mechanically. The enum is the typed surface; the macro
/// bundles serde tag + Clone/PartialEq + Discriminant/IsVariant +
/// TypedDispatcher + register_dispatcher! in one line.
#[derive(thiserror::Error)]
#[gen_macros::fsm(label = "gen.cargo.prefetch-error")]
pub enum PrefetchError {
    #[error("git fetch failed for {url}#{rev}: {reason}")]
    Fetch {
        url: String,
        rev: String,
        reason: String,
    },
    #[error("NAR serialization/hash failed for {url}#{rev}: {reason}")]
    NarHash {
        url: String,
        rev: String,
        reason: String,
    },
    #[error("temp dir creation failed: {reason}")]
    TempDir { reason: String },
    #[error("MockPrefetcher: no mapping registered for {url}#{rev}")]
    MockMappingMissing { url: String, rev: String },
}

// ── Production impl: gix + nix-nar + sha256 ────────────────────────

/// Production prefetcher. `gix` for the clone (pure Rust — no `git`
/// binary), `nix-nar` Encoder streamed into `Sha256` (pure Rust — no
/// `nix` binary, no `nix-prefetch-git` shell script). Every transitive
/// dependency is a crates.io crate compiled into the gen binary; the
/// IFD sandbox needs nothing beyond the `gen` binary itself + cacert
/// for HTTPS.
pub struct GixPrefetcher;

impl GixPrefetcher {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for GixPrefetcher {
    fn default() -> Self {
        Self::new()
    }
}

impl GitPrefetcher for GixPrefetcher {
    fn prefetch(&self, url: &str, rev: &str) -> Result<PrefetchedHash, PrefetchError> {
        // Strip cargo's `?branch=...` / `?tag=...` / `?rev=...` query
        // suffix — git URLs don't carry the query form.
        let clean_url = url.split('?').next().unwrap_or(url);

        let tmp = tempfile::Builder::new()
            .prefix("gen-cargo-prefetch-")
            .tempdir()
            .map_err(|e| PrefetchError::TempDir {
                reason: e.to_string(),
            })?;
        let work = tmp.path();

        gix_fetch_and_checkout(clean_url, rev, work).map_err(|reason| PrefetchError::Fetch {
            url: clean_url.into(),
            rev: rev.into(),
            reason,
        })?;

        // Drop `.git` before NAR-hashing — matches `nix-prefetch-git`
        // default (`deepClone=false`). Without this the digest would
        // include git's mutable index/objects/refs and re-runs would
        // produce different hashes.
        let dot_git = work.join(".git");
        if dot_git.exists() {
            std::fs::remove_dir_all(&dot_git).map_err(|e| PrefetchError::NarHash {
                url: clean_url.into(),
                rev: rev.into(),
                reason: format!("remove .git: {e}"),
            })?;
        }

        let digest = nar_sha256(work).map_err(|reason| PrefetchError::NarHash {
            url: clean_url.into(),
            rev: rev.into(),
            reason,
        })?;
        Ok(PrefetchedHash::from_digest(digest))
    }
}

/// Clone `url` and check out `rev` into `dest`. Uses `gix` for
/// pure-Rust git transport. Returns a stringly-typed error reason so
/// the caller can wrap it in the typed `PrefetchError::Fetch`.
fn gix_fetch_and_checkout(url: &str, rev: &str, dest: &Path) -> Result<(), String> {
    use gix::progress::Discard;

    let interrupt = std::sync::atomic::AtomicBool::new(false);

    let mut prep =
        gix::prepare_clone(url, dest).map_err(|e| format!("prepare_clone: {e}"))?;
    let (mut fetch_prep, _) = prep
        .fetch_then_checkout(Discard, &interrupt)
        .map_err(|e| format!("fetch_then_checkout: {e}"))?;
    let (repo, _) = fetch_prep
        .main_worktree(Discard, &interrupt)
        .map_err(|e| format!("main_worktree: {e}"))?;

    // Resolve `rev` to a commit and update the working tree to it.
    // `rev_parse_single` accepts full/short SHAs, branch names, tags.
    let oid = repo
        .rev_parse_single(rev.as_bytes())
        .map_err(|e| format!("rev_parse_single({rev}): {e}"))?;

    // Find the tree for that commit and check it out into the worktree.
    let commit = repo
        .find_object(oid)
        .map_err(|e| format!("find_object({oid}): {e}"))?
        .try_into_commit()
        .map_err(|e| format!("try_into_commit({oid}): {e}"))?;
    let tree = commit
        .tree()
        .map_err(|e| format!("commit.tree(): {e}"))?;

    // Clear existing worktree contents (the initial clone may have
    // checked out HEAD, which differs from `rev` if HEAD points at a
    // branch tip past `rev`). Repopulate from `tree`.
    clear_worktree(dest)?;
    checkout_tree(&repo, &tree, dest)?;

    Ok(())
}

/// Walk `dest` and delete every entry except `.git`. `gix` leaves the
/// initial-clone HEAD in place; we want to replace it with the tree
/// at the requested rev.
fn clear_worktree(dest: &Path) -> Result<(), String> {
    let entries =
        std::fs::read_dir(dest).map_err(|e| format!("clear_worktree read_dir: {e}"))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("clear_worktree entry: {e}"))?;
        let path = entry.path();
        // Skip `.git` — we still need the index/objects to check out
        // the requested rev. We delete it after NAR-hashing.
        if path.file_name().and_then(|n| n.to_str()) == Some(".git") {
            continue;
        }
        let meta = std::fs::symlink_metadata(&path)
            .map_err(|e| format!("clear_worktree stat {}: {e}", path.display()))?;
        if meta.file_type().is_dir() {
            std::fs::remove_dir_all(&path)
                .map_err(|e| format!("clear_worktree rm dir {}: {e}", path.display()))?;
        } else {
            std::fs::remove_file(&path)
                .map_err(|e| format!("clear_worktree rm file {}: {e}", path.display()))?;
        }
    }
    Ok(())
}

/// Materialize the gix `tree` into `dest`. Recursive; preserves
/// executable bits + symlink targets so the NAR digest matches what
/// `nix-prefetch-git` would have produced.
fn checkout_tree(
    repo: &gix::Repository,
    tree: &gix::Tree<'_>,
    dest: &Path,
) -> Result<(), String> {
    use gix::object::tree::EntryKind;

    for entry in tree.iter() {
        let entry = entry.map_err(|e| format!("tree entry iter: {e}"))?;
        let name = std::str::from_utf8(entry.filename())
            .map_err(|e| format!("non-utf8 filename: {e}"))?;
        let path = dest.join(name);
        let oid = entry.oid().to_owned();

        match entry.mode().kind() {
            EntryKind::Tree => {
                std::fs::create_dir(&path)
                    .map_err(|e| format!("create_dir {}: {e}", path.display()))?;
                let object = repo
                    .find_object(oid)
                    .map_err(|e| format!("find_object(tree): {e}"))?;
                let subtree = object
                    .try_into_tree()
                    .map_err(|e| format!("try_into_tree: {e}"))?;
                checkout_tree(repo, &subtree, &path)?;
            }
            EntryKind::Blob => {
                let object = repo
                    .find_object(oid)
                    .map_err(|e| format!("find_object(blob): {e}"))?;
                let blob = object
                    .try_into_blob()
                    .map_err(|e| format!("try_into_blob: {e}"))?;
                std::fs::write(&path, &blob.data)
                    .map_err(|e| format!("write {}: {e}", path.display()))?;
            }
            EntryKind::BlobExecutable => {
                let object = repo
                    .find_object(oid)
                    .map_err(|e| format!("find_object(blob-exec): {e}"))?;
                let blob = object
                    .try_into_blob()
                    .map_err(|e| format!("try_into_blob: {e}"))?;
                std::fs::write(&path, &blob.data)
                    .map_err(|e| format!("write {}: {e}", path.display()))?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let mut perms = std::fs::metadata(&path)
                        .map_err(|e| format!("metadata {}: {e}", path.display()))?
                        .permissions();
                    perms.set_mode(0o755);
                    std::fs::set_permissions(&path, perms)
                        .map_err(|e| format!("set_permissions {}: {e}", path.display()))?;
                }
            }
            EntryKind::Link => {
                let object = repo
                    .find_object(oid)
                    .map_err(|e| format!("find_object(link): {e}"))?;
                let blob = object
                    .try_into_blob()
                    .map_err(|e| format!("try_into_blob: {e}"))?;
                let target = std::str::from_utf8(&blob.data)
                    .map_err(|e| format!("non-utf8 symlink target: {e}"))?;
                #[cfg(unix)]
                std::os::unix::fs::symlink(target, &path)
                    .map_err(|e| format!("symlink {}: {e}", path.display()))?;
                #[cfg(windows)]
                {
                    let _ = target;
                    return Err(format!(
                        "symlink at {} unsupported on windows (target: {})",
                        path.display(),
                        std::str::from_utf8(&blob.data).unwrap_or("<binary>")
                    ));
                }
            }
            EntryKind::Commit => {
                // gitlink — submodule. nix-prefetch-git without
                // --fetch-submodules leaves an empty directory.
                std::fs::create_dir(&path)
                    .map_err(|e| format!("submodule placeholder {}: {e}", path.display()))?;
            }
        }
    }
    Ok(())
}

/// Stream-encode `path` as a NAR via the battle-tested `nix-nar`
/// crate (same one `sui-compat::nar::NarWriter::write_path` wraps —
/// fleet-aligned semantics) and Sha256 the byte stream.
///
/// Streaming matters: cloned repos can be huge. Materializing the
/// NAR into memory before hashing would balloon RAM use; copying
/// through a hash-writer keeps the working set to the I/O buffer.
fn nar_sha256(path: &Path) -> Result<[u8; 32], String> {
    let encoder =
        nix_nar::Encoder::new(path).map_err(|e| format!("nix-nar Encoder: {e}"))?;
    let mut reader = std::io::BufReader::new(encoder);
    let mut writer = Sha256Writer::new();
    std::io::copy(&mut reader, &mut writer).map_err(|e| format!("nar copy: {e}"))?;
    Ok(writer.finalize())
}

/// `io::Write` adapter that feeds bytes into a `Sha256` hasher. Lets
/// `std::io::copy` (or any other byte-stream consumer) drive the
/// hash without materializing the NAR.
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

/// Test-only mock. Construct, register `(url, rev) → hash` mappings,
/// inject as `&dyn GitPrefetcher`. Every prefetch call MUST hit a
/// registered mapping; an unregistered call returns a typed
/// `PrefetchError::MockMappingMissing` so test failures point at the
/// missing fixture, not at a vague "MockPrefetcher panicked."
#[derive(Default)]
pub struct MockPrefetcher {
    mappings: Mutex<BTreeMap<(String, String), PrefetchedHash>>,
}

impl MockPrefetcher {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a `(url, rev) → hash` mapping.
    pub fn insert(
        &self,
        url: impl Into<String>,
        rev: impl Into<String>,
        hash: PrefetchedHash,
    ) {
        self.mappings
            .lock()
            .expect("MockPrefetcher mutex poisoned")
            .insert((url.into(), rev.into()), hash);
    }
}

impl GitPrefetcher for MockPrefetcher {
    fn prefetch(&self, url: &str, rev: &str) -> Result<PrefetchedHash, PrefetchError> {
        // Strip cargo query suffix — match the production normalization.
        let clean_url = url.split('?').next().unwrap_or(url);
        let key = (clean_url.to_string(), rev.to_string());
        self.mappings
            .lock()
            .expect("MockPrefetcher mutex poisoned")
            .get(&key)
            .cloned()
            .ok_or_else(|| PrefetchError::MockMappingMissing {
                url: clean_url.into(),
                rev: rev.into(),
            })
    }
}

// ── Default factory ────────────────────────────────────────────────

/// Construct the production prefetcher. Wrapped in a factory so
/// downstream consumers don't bind to `GixPrefetcher` directly —
/// trait-dispatched.
#[must_use]
pub fn default_prefetcher() -> Box<dyn GitPrefetcher> {
    Box::new(GixPrefetcher::new())
}

// ── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn sri_encoding_is_canonical() {
        // Zero digest → known SRI string.
        let zero = PrefetchedHash::from_digest([0u8; 32]);
        assert_eq!(
            zero.sri,
            "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
        );
    }

    #[test]
    fn mock_returns_registered_hash() {
        let mock = MockPrefetcher::new();
        let hash = PrefetchedHash::from_digest([1u8; 32]);
        mock.insert("https://example.com/repo", "deadbeef", hash.clone());

        let got = mock.prefetch("https://example.com/repo", "deadbeef").unwrap();
        assert_eq!(got, hash);
    }

    #[test]
    fn mock_strips_cargo_query_suffix() {
        let mock = MockPrefetcher::new();
        let hash = PrefetchedHash::from_digest([2u8; 32]);
        mock.insert("https://example.com/repo", "rev1", hash.clone());

        // Caller uses cargo's `?rev=` form; mock should normalize.
        let got = mock
            .prefetch("https://example.com/repo?rev=rev1", "rev1")
            .unwrap();
        assert_eq!(got, hash);
    }

    #[test]
    fn mock_missing_returns_typed_error() {
        let mock = MockPrefetcher::new();
        let err = mock
            .prefetch("https://example.com/repo", "rev1")
            .unwrap_err();
        matches!(
            err,
            PrefetchError::MockMappingMissing { ref url, ref rev }
                if url == "https://example.com/repo" && rev == "rev1"
        );
    }

    #[test]
    fn nar_sha256_of_empty_dir_is_deterministic() {
        let tmp = tempfile::tempdir().unwrap();
        let first = nar_sha256(tmp.path()).unwrap();
        let second = nar_sha256(tmp.path()).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn nar_sha256_changes_when_file_added() {
        let tmp = tempfile::tempdir().unwrap();
        let empty_digest = nar_sha256(tmp.path()).unwrap();

        fs::write(tmp.path().join("hello.txt"), b"world").unwrap();
        let one_file_digest = nar_sha256(tmp.path()).unwrap();

        assert_ne!(empty_digest, one_file_digest);
    }

    #[test]
    fn nar_sha256_changes_when_contents_change() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("a.txt"), b"alpha").unwrap();
        let a = nar_sha256(tmp.path()).unwrap();

        fs::write(tmp.path().join("a.txt"), b"beta").unwrap();
        let b = nar_sha256(tmp.path()).unwrap();

        assert_ne!(a, b);
    }

    #[test]
    fn nar_sha256_independent_of_directory_walk_order() {
        // NAR canonical order is lexicographic on filename — even if
        // the filesystem reads back in insertion order, the NAR
        // output must sort. Build same logical tree two different
        // creation orders and assert digest equality.
        let tmp_a = tempfile::tempdir().unwrap();
        fs::write(tmp_a.path().join("z.txt"), b"z").unwrap();
        fs::write(tmp_a.path().join("a.txt"), b"a").unwrap();
        let da = nar_sha256(tmp_a.path()).unwrap();

        let tmp_b = tempfile::tempdir().unwrap();
        fs::write(tmp_b.path().join("a.txt"), b"a").unwrap();
        fs::write(tmp_b.path().join("z.txt"), b"z").unwrap();
        let db = nar_sha256(tmp_b.path()).unwrap();

        assert_eq!(da, db);
    }

    #[test]
    fn default_prefetcher_returns_gix() {
        // Smoke test: the factory boxes a GixPrefetcher without
        // panicking. Real network calls live in integration tests.
        let _: Box<dyn GitPrefetcher> = default_prefetcher();
    }
}
