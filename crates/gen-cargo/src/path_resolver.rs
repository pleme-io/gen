//! Typed resolver for external path-deps.
//!
//! When a Cargo.toml declares `path = "../sibling-repo"`, gen-cargo
//! emission detects the workspace-escape and asks the resolver to
//! convert it into a `(git_url, rev)` pair. The trait is the
//! testability seam: real builds use [`GitCliResolver`] (subprocess
//! `git config remote.origin.url` + `git rev-parse HEAD`); tests
//! inject [`MockResolver`] with hand-authored mappings so every
//! emission path is exercised hermetically (no network, no git
//! state).
//!
//! Same shape as the org-level "Typed-Spec Triplet" pattern: the
//! trait IS the contract that lets gen run inside or outside a real
//! git workspace identically.
//!
//! ## Resolution policy
//!
//! Given an absolute directory path that is OUTSIDE the cargo
//! workspace root, the resolver attempts:
//!
//! 1. Read `.git/config` (or `git config remote.origin.url`) to find
//!    the remote URL. Normalize SSH (`git@github.com:o/r.git`) to
//!    HTTPS (`https://github.com/o/r`) so substrate's fetchgit works
//!    in the nix sandbox without an SSH key.
//! 2. Read `HEAD` (or `git rev-parse HEAD`) to find the commit rev.
//!    Falls back to `origin/main`'s rev when HEAD is detached or
//!    points at an unpushed commit.
//! 3. Return `Some((normalized_url, rev_sha))`.
//!
//! Returns `None` when:
//!   - The directory isn't a git repo.
//!   - The repo has no `origin` remote.
//!   - All resolution steps fail.
//!
//! Callers turn `None` into a typed `CargoError::UnresolvableExternalPath`.

use std::path::Path;

/// Typed boundary for resolving an external path-dep into a git
/// source. Implementors return `(remote_url, rev)` for an
/// out-of-workspace absolute directory, or `None` if the directory
/// is not a recognizable git repository.
pub trait PathDepResolver: Send + Sync {
    /// Resolve the given absolute directory to `(git_url, rev)`.
    /// `url` is normalized to HTTPS form so it works hermetically
    /// inside the nix sandbox.
    fn resolve(&self, abs_dir: &Path) -> Option<(String, String)>;
}

/// Production resolver — shells out to `git` to read remote + rev.
#[derive(Debug, Default, Clone, Copy)]
pub struct GitCliResolver;

impl PathDepResolver for GitCliResolver {
    fn resolve(&self, abs_dir: &Path) -> Option<(String, String)> {
        let raw_url = git_remote_url(abs_dir)?;
        let url = normalize_to_https(&raw_url)?;
        let rev = git_head_rev(abs_dir)?;
        Some((url, rev))
    }
}

fn git_remote_url(dir: &Path) -> Option<String> {
    let out = std::process::Command::new("git")
        .args(["-C", dir.to_str()?, "config", "--get", "remote.origin.url"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?;
    let trimmed = s.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn git_head_rev(dir: &Path) -> Option<String> {
    let out = std::process::Command::new("git")
        .args(["-C", dir.to_str()?, "rev-parse", "HEAD"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?;
    let trimmed = s.trim();
    if trimmed.len() == 40 && trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
        Some(trimmed.to_string())
    } else {
        None
    }
}

/// Normalize a git remote URL to the canonical HTTPS form that
/// works inside the nix sandbox. Accepts:
///
///   - `git@github.com:owner/repo.git`  → `https://github.com/owner/repo`
///   - `git@github.com:owner/repo`      → `https://github.com/owner/repo`
///   - `ssh://git@github.com/owner/repo` → `https://github.com/owner/repo`
///   - `https://github.com/owner/repo.git` → `https://github.com/owner/repo`
///   - `https://github.com/owner/repo`  → unchanged
///
/// Returns `None` for non-GitHub URLs (gitlab, gitea, etc.) — those
/// are out of scope for the auto-converter and surface as a typed
/// error pointing the operator at the manual fix. Future work can
/// extend the parser per host.
pub fn normalize_to_https(raw: &str) -> Option<String> {
    let r = raw.trim();
    if let Some(rest) = r.strip_prefix("git@github.com:") {
        return Some(format!("https://github.com/{}", strip_dot_git(rest)));
    }
    if let Some(rest) = r.strip_prefix("ssh://git@github.com/") {
        return Some(format!("https://github.com/{}", strip_dot_git(rest)));
    }
    if let Some(rest) = r.strip_prefix("https://github.com/") {
        return Some(format!("https://github.com/{}", strip_dot_git(rest)));
    }
    None
}

fn strip_dot_git(s: &str) -> &str {
    s.trim_end_matches(".git").trim_end_matches('/')
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test injection: hand-authored mapping from abs dir → (url, rev).
    /// Lets every gen-cargo emission test exercise the external-path
    /// branch hermetically.
    #[derive(Default)]
    pub struct MockResolver {
        pub mappings: std::collections::HashMap<std::path::PathBuf, (String, String)>,
    }

    impl PathDepResolver for MockResolver {
        fn resolve(&self, abs_dir: &Path) -> Option<(String, String)> {
            self.mappings.get(abs_dir).cloned()
        }
    }

    #[test]
    fn normalize_to_https_recognizes_ssh_short_form() {
        assert_eq!(
            normalize_to_https("git@github.com:pleme-io/gen.git").as_deref(),
            Some("https://github.com/pleme-io/gen"),
        );
    }

    #[test]
    fn normalize_to_https_recognizes_ssh_full_form() {
        assert_eq!(
            normalize_to_https("ssh://git@github.com/pleme-io/tatara/").as_deref(),
            Some("https://github.com/pleme-io/tatara"),
        );
    }

    #[test]
    fn normalize_to_https_passthrough_for_canonical() {
        assert_eq!(
            normalize_to_https("https://github.com/pleme-io/shikumi").as_deref(),
            Some("https://github.com/pleme-io/shikumi"),
        );
    }

    #[test]
    fn normalize_to_https_strips_dot_git() {
        assert_eq!(
            normalize_to_https("https://github.com/pleme-io/sui.git").as_deref(),
            Some("https://github.com/pleme-io/sui"),
        );
    }

    #[test]
    fn normalize_to_https_rejects_non_github() {
        assert_eq!(normalize_to_https("https://gitlab.com/x/y"), None);
        assert_eq!(normalize_to_https("git@bitbucket.org:x/y"), None);
    }

    #[test]
    fn mock_resolver_returns_mapped_value() {
        let mut m = MockResolver::default();
        m.mappings.insert(
            std::path::PathBuf::from("/code/gen"),
            ("https://github.com/pleme-io/gen".into(), "abc123".into()),
        );
        assert_eq!(
            m.resolve(Path::new("/code/gen")),
            Some(("https://github.com/pleme-io/gen".into(), "abc123".into())),
        );
        assert_eq!(m.resolve(Path::new("/unknown")), None);
    }
}
