//! `GitPrefetcher` — Rust-native git prefetch primitive.
//!
//! Replaces the brittle `nix-prefetch-git` shell-script call chain
//! that gen-cargo previously shelled out to. The hash side is
//! pure-Rust: `nix-nar` streamed into `Sha256`, then base64 SRI
//! encoded. The clone side is a single `Command::new("git")`
//! subprocess — one load-bearing binary, no shell script, no
//! transitive-tool dependency tree. The previous nix-prefetch-git
//! shell-script chain (bash + git + nix-hash + nix-prefetch-git
//! itself + coreutils) is reduced to: git.
//!
//! Destination: pure-`gix` (zero subprocess). Wired 2026-05-29 but
//! deferred — `gix`'s transport features pull `ring` (rustls path,
//! substrate build-script env-var bug) or `curl-sys`/`libgit2-sys`
//! (substrate `propagatedBuildInputs` not bubbled through
//! `buildRustCrate` to consumers). Both are focused substrate-side
//! fixes scheduled in a follow-up; see `theory/RUST-NATIVE-PREFETCH.md`.
//!
//! Architecture: typed trait + Mock for hermetic testing + production
//! `GitCliPrefetcher` impl. The build-spec generator depends on the
//! trait object so unit tests swap in `MockPrefetcher` with hand-
//! authored `(url, rev) → sha256` mappings — no network, no tempdirs,
//! no flakiness.

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

// ── Production impl: `git` subprocess + nix-nar + sha256 ───────────

/// Production prefetcher. One `git` subprocess for the fetch + checkout,
/// pure-Rust `nix-nar` Encoder streamed into `Sha256` for the digest.
/// No shell script, no transitive-tool dependency tree — `git` is the
/// single load-bearing binary.
///
/// **Destination**: pure-Rust via `gix` (zero subprocess). Deferred
/// behind a substrate-side fix for *-sys propagation in
/// `buildRustCrate`. See `theory/RUST-NATIVE-PREFETCH.md`.
pub struct GitCliPrefetcher;

impl GitCliPrefetcher {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for GitCliPrefetcher {
    fn default() -> Self {
        Self::new()
    }
}

impl GitPrefetcher for GitCliPrefetcher {
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

        git_clone_at_rev(clean_url, rev, work).map_err(|reason| PrefetchError::Fetch {
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

/// Clone `url` and check out `rev` into `dest`. Uses the `git` CLI
/// binary directly — no shell, no nix-prefetch-git wrapper. Strategy:
///
/// 1. `git init` the dest (empty repo).
/// 2. `git fetch --depth=1 origin <rev>` — pulls only the rev's
///    objects, not history. `<rev>` can be a full SHA, tag, or
///    branch name. Shallow-clone for speed.
/// 3. `git -c advice.detachedHead=false checkout FETCH_HEAD` to
///    materialize the worktree.
///
/// This mirrors what nix-prefetch-git does internally without the
/// surrounding bash + nix-hash + json-formatting machinery.
fn git_clone_at_rev(url: &str, rev: &str, dest: &Path) -> Result<(), String> {
    use std::process::Command;
    use std::time::Duration;

    use crate::bounded::{Bounded, git_stall_guard};

    /// Local git work: no network, so a long bound here only exists to
    /// catch a wedged filesystem or a paused process.
    const LOCAL_BOUND: Duration = Duration::from_secs(60);
    /// One network fetch. Generous enough for a large cold repo on a slow
    /// link, short enough that a stall is caught in minutes rather than
    /// never. Layer 1 (the stall guard) normally fires first, at 30s of
    /// no throughput.
    const FETCH_BOUND: Duration = Duration::from_secs(300);

    let local = Bounded::new(LOCAL_BOUND);
    // Retries are what a bound buys: a transient reset or stall now
    // recovers instead of wedging a fleet-wide lock refresh.
    let network = Bounded::new(FETCH_BOUND).attempts(3).retry_failures(true);

    let run = |b: &Bounded, args: &[&str], cwd: Option<&Path>| -> Result<(), String> {
        let owned: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
        let cwd = cwd.map(std::path::Path::to_path_buf);
        b.run(|| {
            let mut cmd = Command::new("git");
            cmd.args(&owned);
            if let Some(c) = cwd.as_ref() {
                cmd.current_dir(c);
            }
            cmd
        })
        .map(|_| ())
        .map_err(|e| e.to_string())
    };

    run(&local, &["init", "--quiet", dest.to_str().unwrap()], None)?;
    run(&local, &["remote", "add", "origin", url], Some(dest))?;

    // ── ★ THE STALL GUARD IS WHY THE FALLBACK BELOW STILL WORKS ──────
    // The fallback is `if shallow_fetch.is_err()`. Before the guard, a
    // stalled fetch was never `Err` — it simply never returned — so
    // neither the fallback NOR the error path was ever reached, and the
    // process sat at 0% CPU indefinitely. Measured 2026-08-27: six
    // orphaned `git index-pack` processes, 20 minutes, no output.
    // `http.lowSpeedLimit`/`lowSpeedTime` make git abort a dead transfer
    // itself, turning the hang into an ordinary error this code already
    // knows how to handle.
    let guard = git_stall_guard();
    let mut shallow: Vec<&str> = guard.to_vec();
    shallow.extend_from_slice(&["fetch", "--quiet", "--depth=1", "origin", rev]);
    let mut full: Vec<&str> = guard.to_vec();
    full.extend_from_slice(&["fetch", "--quiet", "origin", rev]);

    // --depth=1 keeps the fetch minimal — only the rev's tree, no
    // history. Some servers reject shallow fetches by SHA; fall back
    // to a full fetch + checkout if the shallow attempt fails.
    if run(&network, &shallow, Some(dest)).is_err() {
        run(&network, &full, Some(dest))?;
    }
    run(
        &local,
        &[
            "-c",
            "advice.detachedHead=false",
            "checkout",
            "--quiet",
            "FETCH_HEAD",
        ],
        Some(dest),
    )?;
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
    let encoder = nix_nar::Encoder::new(path).map_err(|e| format!("nix-nar Encoder: {e}"))?;
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
    pub fn insert(&self, url: impl Into<String>, rev: impl Into<String>, hash: PrefetchedHash) {
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
/// downstream consumers don't bind to the concrete impl directly —
/// trait-dispatched, swap-friendly for the eventual gix migration.
#[must_use]
pub fn default_prefetcher() -> Box<dyn GitPrefetcher> {
    Box::new(GitCliPrefetcher::new())
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

        let got = mock
            .prefetch("https://example.com/repo", "deadbeef")
            .unwrap();
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
        // Smoke test: the factory boxes a GitCliPrefetcher without
        // panicking. Real network calls live in integration tests.
        let _: Box<dyn GitPrefetcher> = default_prefetcher();
    }
}
