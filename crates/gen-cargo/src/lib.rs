//! `gen-cargo` — Cargo adapter for the `gen` ecosystem.
//!
//! Reads a Cargo workspace (or single-crate repo) + its lockfile and
//! emits a typed [`gen_types::Manifest`]. The cargo half of the
//! universal package-manager engine — one of N adapters that share a
//! single typed IR. See `theory/GEN.md` for the design.
//!
//! ```no_run
//! use std::path::Path;
//! let manifest = gen_cargo::parse(Path::new("/path/to/workspace")).unwrap();
//! assert!(manifest.package_count() >= 1);
//! ```

pub mod adapter;
pub mod build_spec;
pub mod convert;
pub mod diagnostics;
pub mod ecosystem_impl;
pub mod error;
pub mod features;
pub mod fleet_commit;
pub mod fleet_sweep;
pub mod invariants;
pub mod lock_lifecycle;
pub mod path_resolver;
pub mod platform_features;
pub mod quirks;
pub mod raw;

pub use adapter::{ctx_for, CargoAdapter};
pub use error::{CargoError, Result};

use std::path::{Path, PathBuf};

use gen_types::Manifest;

/// Parse the Cargo workspace (or single-crate repo) rooted at `root`
/// into a typed [`Manifest`]. Reads `<root>/Cargo.toml`, every member
/// crate's `Cargo.toml`, and (if present) `<root>/Cargo.lock`.
///
/// # Errors
///
/// Returns a typed [`CargoError`] for IO failures, TOML parse errors,
/// missing workspace members, or malformed version requirements.
pub fn parse(root: &Path) -> Result<Manifest> {
    let root_manifest_path = root.join("Cargo.toml");
    let root_raw = read_cargo_toml(&root_manifest_path)?;

    let mut packages = Vec::new();

    if let Some(_pkg) = &root_raw.package {
        // Single-crate or workspace-root-also-a-package case.
        let pkg = convert::convert_package(
            &root_raw,
            &root_manifest_path,
            workspace_pkg(&root_raw),
            workspace_deps(&root_raw),
        )?;
        packages.push(pkg);
    }

    if let Some(ws) = &root_raw.workspace {
        for member in &ws.members {
            // Glob support (`crates/*`) is intentionally deferred —
            // the consumer can pre-expand globs; the engine never
            // shells out. Members with `*` are skipped with a
            // diagnostic carried through MissingWorkspaceMember.
            if member.contains('*') {
                if let Some(expanded) = expand_glob(root, member) {
                    for mpath in expanded {
                        packages.push(parse_member(root, &mpath, &root_raw)?);
                    }
                    continue;
                }
            }
            let mpath = PathBuf::from(member);
            packages.push(parse_member(root, &mpath, &root_raw)?);
        }
    }

    let lockfile = match read_optional(&root.join("Cargo.lock"))? {
        Some(text) => {
            let parsed: raw::CargoLock =
                toml::from_str(&text).map_err(|source| CargoError::Toml {
                    path: root.join("Cargo.lock"),
                    source,
                })?;
            Some(convert::convert_lockfile(&parsed, &root.join("Cargo.lock"))?)
        }
        None => None,
    };

    Ok(convert::build_manifest(root, &root_raw, packages, lockfile))
}

fn parse_member(
    root: &Path,
    member_rel: &Path,
    workspace_root_raw: &raw::CargoToml,
) -> Result<gen_types::Package> {
    let path = root.join(member_rel).join("Cargo.toml");
    let raw = read_cargo_toml(&path).map_err(|e| match e {
        CargoError::Io { .. } => CargoError::MissingWorkspaceMember {
            root: root.to_path_buf(),
            member: member_rel.display().to_string(),
        },
        other => other,
    })?;
    convert::convert_package(
        &raw,
        &path,
        workspace_pkg(workspace_root_raw),
        workspace_deps(workspace_root_raw),
    )
}

fn workspace_pkg(raw: &raw::CargoToml) -> Option<&raw::RawPackage> {
    raw.workspace.as_ref().and_then(|w| w.package.as_ref())
}

fn workspace_deps(raw: &raw::CargoToml) -> &indexmap::IndexMap<String, raw::RawDep> {
    static EMPTY: std::sync::OnceLock<indexmap::IndexMap<String, raw::RawDep>> =
        std::sync::OnceLock::new();
    raw.workspace
        .as_ref()
        .map(|w| &w.dependencies)
        .unwrap_or_else(|| EMPTY.get_or_init(indexmap::IndexMap::new))
}

fn read_cargo_toml(path: &Path) -> Result<raw::CargoToml> {
    let text = std::fs::read_to_string(path).map_err(|source| CargoError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    toml::from_str(&text).map_err(|source| CargoError::Toml {
        path: path.to_path_buf(),
        source,
    })
}

fn read_optional(path: &Path) -> Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(CargoError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// Minimal glob expansion for `crates/*` / `crates/**` shapes — no
/// regex, no external dep. Reads the parent dir and filters by the
/// segment after the `*`. Returns `None` if the pattern shape is
/// unsupported (caller treats it as a literal path).
fn expand_glob(root: &Path, pattern: &str) -> Option<Vec<PathBuf>> {
    let (prefix, rest) = pattern.split_once('*')?;
    if !rest.is_empty() && rest != "/" {
        return None; // unsupported tail
    }
    // Cargo glob shapes seen in the fleet:
    //   "crates/*"        — dir-prefix glob (matches dirs under crates/)
    //   "ratatui-*"       — file-prefix glob (matches dirs in root with this prefix)
    // Distinguish by whether prefix ends with '/' (dir) or a name segment.
    let (search_dir, name_prefix) =
        if prefix.is_empty() || prefix.ends_with('/') {
            (root.join(prefix.trim_end_matches('/')), String::new())
        } else if let Some(slash) = prefix.rfind('/') {
            (root.join(&prefix[..slash]), prefix[slash + 1..].to_string())
        } else {
            (root.to_path_buf(), prefix.to_string())
        };
    let mut out = Vec::new();
    let entries = std::fs::read_dir(&search_dir).ok()?;
    for e in entries.flatten() {
        let path = e.path();
        if !path.is_dir() || !path.join("Cargo.toml").exists() {
            continue;
        }
        let fname = path.file_name()?.to_string_lossy().into_owned();
        if !name_prefix.is_empty() && !fname.starts_with(&name_prefix) {
            continue;
        }
        if let Ok(rel) = path.strip_prefix(root) {
            out.push(rel.to_path_buf());
        }
    }
    out.sort();
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(path: &Path, body: &str) {
        if let Some(p) = path.parent() {
            fs::create_dir_all(p).unwrap();
        }
        fs::write(path, body).unwrap();
    }

    #[test]
    fn parses_single_crate_repo() {
        let dir = tempfile_dir();
        write(
            &dir.join("Cargo.toml"),
            r#"
[package]
name = "demo"
version = "0.2.1"
edition = "2024"
license = "MIT"
description = "demo crate"

[dependencies]
serde = "1.0"
serde_json = { version = "1", features = ["preserve_order"] }
indexmap = { version = "2", default-features = false }
"#,
        );
        let m = parse(&dir).unwrap();
        assert_eq!(m.package_count(), 1);
        let p = m.find_package("demo").unwrap();
        assert_eq!(p.version, gen_types::Version::new(0, 2, 1));
        assert_eq!(p.license.as_deref(), Some("MIT"));
        assert_eq!(p.dependencies.len(), 3);
        let s = p.dependencies.iter().find(|d| d.name == "serde").unwrap();
        assert!(matches!(s.kind, gen_types::DependencyKind::Direct));
        assert_eq!(s.constraint.native_syntax.as_deref(), Some("1.0"));
        let sj = p.dependencies.iter().find(|d| d.name == "serde_json").unwrap();
        assert_eq!(sj.features_enabled, vec!["preserve_order".to_string()]);
        let im = p.dependencies.iter().find(|d| d.name == "indexmap").unwrap();
        assert!(!im.default_features);
    }

    #[test]
    fn parses_workspace_with_member_inheritance() {
        let dir = tempfile_dir();
        write(
            &dir.join("Cargo.toml"),
            r#"
[workspace]
members = ["crates/a", "crates/b"]

[workspace.package]
version = "0.3.0"
license = "MIT"
edition = "2024"

[workspace.dependencies]
serde = "1.0"
shared = { version = "0.5", features = ["foo"] }
"#,
        );
        write(
            &dir.join("crates/a/Cargo.toml"),
            r#"
[package]
name = "a"
version.workspace = true
license.workspace = true
edition.workspace = true

[dependencies]
serde = { workspace = true }
"#,
        );
        write(
            &dir.join("crates/b/Cargo.toml"),
            r#"
[package]
name = "b"
version = "0.9.0"
edition = "2024"
license = "MIT"

[dependencies]
shared = { workspace = true, features = ["bar"] }
"#,
        );
        let m = parse(&dir).unwrap();
        assert_eq!(m.package_count(), 2);
        let a = m.find_package("a").unwrap();
        assert_eq!(a.version, gen_types::Version::new(0, 3, 0));
        assert_eq!(a.license.as_deref(), Some("MIT"));
        assert_eq!(a.dependencies[0].name, "serde");

        let b = m.find_package("b").unwrap();
        assert_eq!(b.version, gen_types::Version::new(0, 9, 0));
        let shared = b.dependencies.iter().find(|d| d.name == "shared").unwrap();
        // inherited foo + local bar
        assert!(shared.features_enabled.contains(&"foo".to_string()));
        assert!(shared.features_enabled.contains(&"bar".to_string()));
    }

    #[test]
    fn parses_target_cfg_dependencies() {
        let dir = tempfile_dir();
        write(
            &dir.join("Cargo.toml"),
            r#"
[package]
name = "p"
version = "0.1.0"
edition = "2024"

[dependencies]

[target.'cfg(unix)'.dependencies]
nix = "0.27"

[target.'cfg(windows)'.dev-dependencies]
winapi = "0.3"
"#,
        );
        let m = parse(&dir).unwrap();
        let p = m.find_package("p").unwrap();
        let nix = p.dependencies.iter().find(|d| d.name == "nix").unwrap();
        let pred = nix.target_predicate.as_ref().unwrap();
        assert!(matches!(pred, gen_types::TargetPredicate::CargoCfg { expr } if expr.contains("unix")));
        let winapi = p.dependencies.iter().find(|d| d.name == "winapi").unwrap();
        assert!(matches!(winapi.kind, gen_types::DependencyKind::Dev));
    }

    #[test]
    fn parses_git_dependency_with_rev() {
        let dir = tempfile_dir();
        write(
            &dir.join("Cargo.toml"),
            r#"
[package]
name = "p"
version = "0.1.0"
edition = "2024"

[dependencies]
foo = { git = "https://github.com/x/y", rev = "abc123" }
bar = { git = "https://github.com/x/z", branch = "main" }
"#,
        );
        let m = parse(&dir).unwrap();
        let p = m.find_package("p").unwrap();
        let foo = p.dependencies.iter().find(|d| d.name == "foo").unwrap();
        let src = foo.source_override.as_ref().unwrap();
        match src {
            gen_types::PackageSource::Git { url, rev, .. } => {
                assert_eq!(url, "https://github.com/x/y");
                assert_eq!(rev, "abc123");
            }
            other => panic!("expected Git, got {other:?}"),
        }
        let bar = p.dependencies.iter().find(|d| d.name == "bar").unwrap();
        match bar.source_override.as_ref().unwrap() {
            gen_types::PackageSource::Git { rev, .. } => assert_eq!(rev, "main"),
            other => panic!("expected Git, got {other:?}"),
        }
    }

    #[test]
    fn parses_optional_dep_as_optional_kind() {
        let dir = tempfile_dir();
        write(
            &dir.join("Cargo.toml"),
            r#"
[package]
name = "p"
version = "0.1.0"
edition = "2024"

[dependencies]
opt = { version = "1", optional = true }
"#,
        );
        let m = parse(&dir).unwrap();
        let p = m.find_package("p").unwrap();
        let opt = p.dependencies.iter().find(|d| d.name == "opt").unwrap();
        assert!(matches!(opt.kind, gen_types::DependencyKind::Optional));
    }

    #[test]
    fn parses_features_block() {
        let dir = tempfile_dir();
        write(
            &dir.join("Cargo.toml"),
            r#"
[package]
name = "p"
version = "0.1.0"
edition = "2024"

[dependencies]

[features]
default = ["std"]
std = []
extra = ["serde/derive", "dep:opt"]
"#,
        );
        let m = parse(&dir).unwrap();
        let p = m.find_package("p").unwrap();
        assert_eq!(p.features.len(), 3);
        let extra = p.features.iter().find(|f| f.name == "extra").unwrap();
        assert!(
            extra
                .implies
                .iter()
                .any(|r| matches!(r, gen_types::FeatureRef::Namespaced { package, feature }
                    if package == "serde" && feature == "derive"))
        );
        assert!(
            extra
                .implies
                .iter()
                .any(|r| matches!(r, gen_types::FeatureRef::DepActivation { dep_name }
                    if dep_name == "opt"))
        );
    }

    #[test]
    fn parses_version_range_to_range_spec() {
        let dir = tempfile_dir();
        write(
            &dir.join("Cargo.toml"),
            r#"
[package]
name = "p"
version = "0.1.0"
edition = "2024"

[dependencies]
foo = ">=1.2.3, <2.0.0"
"#,
        );
        let m = parse(&dir).unwrap();
        let p = m.find_package("p").unwrap();
        let foo = p.dependencies.iter().find(|d| d.name == "foo").unwrap();
        match &foo.constraint.spec {
            gen_types::ConstraintSpec::Range {
                lower_inclusive,
                upper_exclusive,
            } => {
                assert_eq!(*lower_inclusive, gen_types::Version::new(1, 2, 3));
                assert_eq!(*upper_exclusive, gen_types::Version::new(2, 0, 0));
            }
            other => panic!("expected Range, got {other:?}"),
        }
    }

    #[test]
    fn parses_lockfile_when_present() {
        let dir = tempfile_dir();
        write(
            &dir.join("Cargo.toml"),
            r#"
[package]
name = "demo"
version = "0.1.0"
edition = "2024"
"#,
        );
        write(
            &dir.join("Cargo.lock"),
            r#"
version = 3

[[package]]
name = "demo"
version = "0.1.0"

[[package]]
name = "serde"
version = "1.0.228"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "0000000000000000000000000000000000000000000000000000000000000001"
dependencies = ["serde_derive"]

[[package]]
name = "serde_derive"
version = "1.0.228"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "0000000000000000000000000000000000000000000000000000000000000002"
"#,
        );
        let m = parse(&dir).unwrap();
        let lock = m.lockfile.as_ref().unwrap();
        assert_eq!(lock.resolved.len(), 3);
        let serde_entry = lock.resolved.get("serde/1.0.228").unwrap();
        match &serde_entry.source {
            gen_types::PackageSource::Registry {
                registry,
                integrity_hash,
                ..
            } => {
                assert_eq!(*registry, gen_types::Registry::CratesIo);
                assert!(integrity_hash.as_ref().unwrap().starts_with("sha256:"));
            }
            other => panic!("expected Registry source, got {other:?}"),
        }
        assert_eq!(serde_entry.resolved_dependencies.len(), 1);
        assert_eq!(serde_entry.resolved_dependencies[0].name, "serde_derive");
        assert_ne!(lock.content_addressed_hash, gen_types::ContentHash::genesis());
    }

    #[test]
    fn glob_expands_workspace_members() {
        let dir = tempfile_dir();
        write(
            &dir.join("Cargo.toml"),
            r#"
[workspace]
members = ["crates/*"]
"#,
        );
        write(
            &dir.join("crates/alpha/Cargo.toml"),
            r#"
[package]
name = "alpha"
version = "0.1.0"
edition = "2024"
"#,
        );
        write(
            &dir.join("crates/beta/Cargo.toml"),
            r#"
[package]
name = "beta"
version = "0.1.0"
edition = "2024"
"#,
        );
        let m = parse(&dir).unwrap();
        assert_eq!(m.package_count(), 2);
        assert!(m.find_package("alpha").is_some());
        assert!(m.find_package("beta").is_some());
    }

    #[test]
    fn missing_workspace_member_surfaces_typed_error() {
        let dir = tempfile_dir();
        write(
            &dir.join("Cargo.toml"),
            r#"
[workspace]
members = ["does/not/exist"]
"#,
        );
        let e = parse(&dir).unwrap_err();
        match e {
            CargoError::MissingWorkspaceMember { member, .. } => {
                assert_eq!(member, "does/not/exist");
            }
            other => panic!("expected MissingWorkspaceMember, got {other:?}"),
        }
    }

    fn tempfile_dir() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let base = std::env::temp_dir().join(format!(
            "gen-cargo-test-{}-{}-{:?}",
            std::process::id(),
            n,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        base
    }
}
