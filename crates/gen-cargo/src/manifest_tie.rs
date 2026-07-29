//! The D2 freshness tie's **subject set** — what a tie must hash for the
//! claim *"this committed artifact describes this build"* to be TRUE.
//!
//! ## Why this module exists
//!
//! The D2 tie was `sha256(Cargo.lock)` and nothing else. That is an
//! **under-scoped guard**: it runs, it is reached, its verdict is honored —
//! and its subject set is too narrow for the claim it makes. `Cargo.lock`
//! pins *which packages at which versions from which sources*. It does NOT
//! pin the facts the gen-delta exists to carry: resolved **features**,
//! `default-features` / `optional` on each edge, `edition`, `links`,
//! `build`, `[lib]` / `[[bin]]` targets, `proc-macro`, and
//! `[package.metadata.pleme]`. Every one of those is declared in a
//! **workspace-local `Cargo.toml`**, and every one of them can change while
//! `Cargo.lock` stays byte-identical.
//!
//! Consequence, measured not theorized: flipping a feature off
//! (`default = ["extra"]` → `default = []`) leaves `Cargo.lock` byte-identical
//! and changes the delta's resolved feature set — and the tie stayed green
//! over a delta that no longer described the build.
//!
//! ## The subject set, stated
//!
//! A verdict must be derived from the subject set it claims about
//! (`theory/UNREPRESENTABILITY.md` §II.4). The tie's subject set is the
//! **pair**:
//!
//! 1. `sha256(Cargo.lock)` — the resolution pin (membership + versions +
//!    sources + registry checksums + git revs).
//! 2. `{ workspace-root-relative path → sha256(bytes) }` for every
//!    **workspace-local manifest** — the workspace root `Cargo.toml` plus
//!    the `Cargo.toml` of every path-source package in the resolve graph.
//!
//! The two halves **compose** to cover the whole set, and the composition is
//! the load-bearing argument:
//!
//! * **Membership** of the local-manifest set is already pinned by half 1 —
//!   every workspace member and path dep has a `[[package]]` entry in
//!   `Cargo.lock`, so adding or removing one moves the lock and half 1 goes
//!   red before half 2 is even consulted.
//! * **Content** of those manifests is pinned by half 2 — which is exactly
//!   what the lock cannot express.
//!
//! So half 2 does not need to independently re-enumerate the workspace to be
//! sound; it only needs to cover content drift of a membership half 1 already
//! fixes. That is why a recorded map is not a self-certifying subject set here.
//!
//! ## What is deliberately EXCLUDED (an over-wide tie is its own defect)
//!
//! A tie that churns on unrelated edits trains operators to regenerate
//! blindly, which destroys the gate as surely as under-scoping does. The
//! boundary:
//!
//! * **Rust source files** — do not participate in resolution. Hashing them
//!   would flip the tie on every commit.
//! * **Registry / git dependency manifests** — pinned transitively by
//!   `Cargo.lock` (`checksum` for registry tarballs, `#rev` for git). Their
//!   feature tables cannot change without moving the lock.
//! * **gen's own version / `FLEET_TARGETS` / encoder logic** — carried by the
//!   artifact's `schema_version`, which the consumer already gates decode on.
//!   Folding it into the tie would invalidate every committed delta on every
//!   gen release: the over-wide-churn defect in its purest form.
//! * **`rust-toolchain.toml` / the cargo binary** — a toolchain fact, not a
//!   workspace declaration. A resolver-behaviour change between cargo
//!   versions is a `schema_version`-class concern. **Named residual risk.**
//! * **`.cargo/config.toml`** — `[patch]` / `[replace]` edits change the
//!   resolution and therefore move `Cargo.lock`; `build.target` is irrelevant
//!   to a spec emitted for all six `FLEET_TARGETS`; source replacement
//!   (vendoring) does not change resolution content. **Named residual risk.**
//! * **Manifests outside the workspace root directory** (an `../sibling`
//!   path dep) — the Nix consumer only ever sees the workspace source tree,
//!   so such a manifest is outside what any of this can prove, and requiring
//!   it present would make `gen confirm` fail spuriously in every eval
//!   sandbox. **Named residual risk**; [`ManifestTie::Match::verified`]
//!   carries the covered count so the subject-set size stays legible.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Workspace-root-relative manifest path → lowercase-hex SHA-256 of that
/// file's bytes. Each entry is exactly `builtins.hashFile "sha256"` of the
/// same path, so the Nix consumer verifies it with the primitive D2 already
/// uses — the same mechanism repeated N times, no second implementation of
/// any rule (which is the drift class this contract exists to kill).
pub type ManifestDigests = BTreeMap<String, String>;

/// One manifest whose recorded digest no longer matches the tree.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManifestDrift {
    /// Workspace-root-relative path, as recorded.
    pub path: String,
    /// The digest the committed artifact recorded.
    pub expected: String,
    /// The digest the file hashes to now. `None` = the manifest is gone,
    /// which is drift, never a skip (fail-closed).
    pub actual: Option<String>,
}

/// Verdict of re-verifying a recorded [`ManifestDigests`] against the tree.
///
/// A sum, not a bool: `Unrecorded` is a genuinely distinct state from
/// `Drifted` and must never be collapsed into "fine". Every non-`Match` arm
/// is a failure to *prove* freshness.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "tie", rename_all = "kebab-case")]
pub enum ManifestTie {
    /// Every recorded manifest re-hashes to its recorded digest. Carries the
    /// **witness of the derivation**: how many manifests the verdict is
    /// actually about. A `Match` is meaningless without it — see
    /// `UNREPRESENTABILITY.md` §II.3 (a guard over an empty subject set is an
    /// assertion wearing a check's clothes), which is why [`verify`] returns
    /// [`ManifestTie::Unrecorded`] rather than `Match { verified: 0 }`.
    Match { verified: usize },
    /// At least one recorded manifest changed or vanished. Itemized.
    Drifted { changed: Vec<ManifestDrift> },
    /// The artifact records no manifest digests at all — a pre-widening
    /// (delta schema v1 / spec schema v11) artifact. Freshness of the
    /// resolver facts is **unprovable**, which is a distinct claim from
    /// "stale" and from "fresh", and is reported as neither.
    Unrecorded,
}

impl ManifestTie {
    /// True unless the recorded manifests are PROVEN to match the tree.
    /// `Unrecorded` is not proof, so it is not a match.
    #[must_use]
    pub fn is_match(&self) -> bool {
        matches!(self, ManifestTie::Match { .. })
    }
}

/// Lowercase-hex render of a digest. A fixed-alphabet encoder rather than a
/// `format!()` call — `format!()` is banned fleet-wide (★★ TYPED EMISSION);
/// this is the typed surface for the one encoding this module emits.
fn to_hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

/// SHA-256 of a file's bytes, lowercase hex. `None` when the file cannot be
/// read — callers treat that as drift, never as a skip.
///
/// Identical to `builtins.hashFile "sha256" <path>` and to
/// `build_spec::hash_cargo_lock`'s digest: one hash, both languages compute
/// it the same way.
#[must_use]
pub fn hash_file_sha256(path: &Path) -> Option<String> {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(path).ok()?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Some(to_hex_lower(&hasher.finalize()))
}

/// Relativize an absolute manifest path against the workspace root, keeping
/// only paths that live **under** the root. Returns `None` for an escaping
/// path (`../sibling/Cargo.toml`) — see the module doc's named residual.
fn relative_under_root(abs: &Path, root: &Path) -> Option<String> {
    let rel = abs.strip_prefix(root).ok()?;
    let s = rel.to_str()?;
    if s.is_empty() {
        return None;
    }
    // Normalize to forward slashes so the recorded key is byte-stable across
    // build hosts (the same discipline as the spec's `relative_path`).
    Some(s.replace('\\', "/"))
}

/// Hash every workspace-local manifest into the recorded map.
///
/// `manifests` is the absolute path of each candidate `Cargo.toml`; the
/// workspace root's own manifest MUST be among them (the caller has already
/// proven it exists). Unreadable or escaping paths are dropped here — an
/// unreadable manifest cannot be part of a spec that was just successfully
/// generated from it.
#[must_use]
pub fn collect<'a, I>(root: &Path, manifests: I) -> ManifestDigests
where
    I: IntoIterator<Item = &'a Path>,
{
    let mut out = ManifestDigests::new();
    for abs in manifests {
        let Some(rel) = relative_under_root(abs, root) else {
            continue;
        };
        if let Some(digest) = hash_file_sha256(abs) {
            out.insert(rel, digest);
        }
    }
    out
}

/// Re-verify a recorded digest map against the tree at `root`. Pure file I/O
/// — no cargo, no network, no `~/.cargo`; safe inside a Nix `runCommand` /
/// `nix flake check` sandbox.
#[must_use]
pub fn verify(root: &Path, recorded: &ManifestDigests) -> ManifestTie {
    if recorded.is_empty() {
        return ManifestTie::Unrecorded;
    }
    let mut changed: Vec<ManifestDrift> = Vec::new();
    for (rel, expected) in recorded {
        let actual = hash_file_sha256(&root.join(rel));
        if actual.as_deref() != Some(expected.as_str()) {
            changed.push(ManifestDrift {
                path: rel.clone(),
                expected: expected.clone(),
                actual,
            });
        }
    }
    if changed.is_empty() {
        ManifestTie::Match {
            verified: recorded.len(),
        }
    } else {
        ManifestTie::Drifted { changed }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(tag: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static C: AtomicU64 = AtomicU64::new(0);
        let n = C.fetch_add(1, Ordering::Relaxed);
        let mut name = String::from("gen-manifest-tie-");
        name.push_str(tag);
        name.push('-');
        name.push_str(&std::process::id().to_string());
        name.push('-');
        name.push_str(&n.to_string());
        let p = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn hex_is_lowercase_and_matches_known_sha256() {
        let dir = tmpdir("hex");
        std::fs::write(dir.join("f"), b"abc").unwrap();
        // Well-known SHA-256("abc").
        assert_eq!(
            hash_file_sha256(&dir.join("f")).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_recorded_map_is_unrecorded_never_a_vacuous_match() {
        let dir = tmpdir("empty");
        // The whole point: a guard over zero subjects must NOT report the
        // strongest verdict. `Match { verified: 0 }` is unreachable.
        assert_eq!(verify(&dir, &ManifestDigests::new()), ManifestTie::Unrecorded);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn collect_then_verify_round_trips() {
        let dir = tmpdir("rt");
        std::fs::create_dir_all(dir.join("crates/a")).unwrap();
        std::fs::write(dir.join("Cargo.toml"), b"[workspace]\n").unwrap();
        std::fs::write(dir.join("crates/a/Cargo.toml"), b"[package]\n").unwrap();
        let digests = collect(
            &dir,
            [
                dir.join("Cargo.toml").as_path(),
                dir.join("crates/a/Cargo.toml").as_path(),
            ],
        );
        assert_eq!(digests.len(), 2);
        assert!(digests.contains_key("Cargo.toml"));
        assert!(digests.contains_key("crates/a/Cargo.toml"));
        assert_eq!(verify(&dir, &digests), ManifestTie::Match { verified: 2 });
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_one_byte_manifest_edit_is_itemized_drift() {
        let dir = tmpdir("drift");
        std::fs::write(dir.join("Cargo.toml"), b"[package]\n").unwrap();
        let digests = collect(&dir, [dir.join("Cargo.toml").as_path()]);
        std::fs::write(dir.join("Cargo.toml"), b"[package]\n\n").unwrap();
        match verify(&dir, &digests) {
            ManifestTie::Drifted { changed } => {
                assert_eq!(changed.len(), 1);
                assert_eq!(changed[0].path, "Cargo.toml");
                assert!(changed[0].actual.is_some());
                assert_ne!(changed[0].actual.as_deref(), Some(changed[0].expected.as_str()));
            }
            other => panic!("expected Drifted, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_vanished_manifest_is_drift_not_a_skip() {
        let dir = tmpdir("gone");
        std::fs::write(dir.join("Cargo.toml"), b"[package]\n").unwrap();
        let digests = collect(&dir, [dir.join("Cargo.toml").as_path()]);
        std::fs::remove_file(dir.join("Cargo.toml")).unwrap();
        match verify(&dir, &digests) {
            ManifestTie::Drifted { changed } => {
                assert_eq!(changed.len(), 1);
                assert_eq!(changed[0].actual, None, "missing file must be fail-closed");
            }
            other => panic!("expected Drifted for a missing manifest, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn escaping_path_deps_are_excluded_from_the_subject_set() {
        // Named residual: an `../sibling/Cargo.toml` is outside the source
        // tree the Nix consumer sees, so it is not recorded — and requiring
        // it would make every eval-sandbox check fail spuriously.
        let dir = tmpdir("escape");
        let sibling = dir.join("sibling");
        std::fs::create_dir_all(&sibling).unwrap();
        std::fs::write(sibling.join("Cargo.toml"), b"[package]\n").unwrap();
        let ws = dir.join("ws");
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::write(ws.join("Cargo.toml"), b"[workspace]\n").unwrap();
        let digests = collect(
            &ws,
            [
                ws.join("Cargo.toml").as_path(),
                sibling.join("Cargo.toml").as_path(),
            ],
        );
        assert_eq!(digests.len(), 1, "only the under-root manifest is recorded");
        assert!(digests.contains_key("Cargo.toml"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
