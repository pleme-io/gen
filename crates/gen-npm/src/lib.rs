//! `gen-npm` — npm/pnpm/yarn adapter for the `gen` ecosystem.
//!
//! Reads a directory containing `package.json` + (optionally)
//! `package-lock.json` or `pnpm-lock.yaml` and emits a typed
//! [`gen_types::Manifest`]. Same shape as `gen-cargo`; sibling
//! adapter for the npm cohort.

pub mod error;
pub mod raw;

pub use error::{NpmError, Result};

use std::path::{Path, PathBuf};

use indexmap::IndexMap;

use gen_types::{
    BuildStep, ConstraintSpec, ContentHash, Dependency, DependencyKind, Feature, FeatureRef,
    Lockfile, Manifest, Package, PackageId, PackageSource, Registry, ResolvedPackage, Version,
    VersionConstraint, Workspace,
};

/// Parse the npm workspace (or single-package repo) rooted at `root`.
///
/// # Errors
/// IO + JSON/YAML parse errors land as typed [`NpmError`] variants.
pub fn parse(root: &Path) -> Result<Manifest> {
    let pkg_json_path = root.join("package.json");
    let raw_pkg = read_package_json(&pkg_json_path)?;

    let mut packages = Vec::new();
    let root_pkg = convert_package(&raw_pkg, &pkg_json_path)?;
    let root_name = root_pkg.name.clone();
    packages.push(root_pkg);

    // npm workspaces: workspaces field can be an array of paths or
    // globs, OR an object with `packages` array. Handle both.
    let member_globs = raw_pkg.workspaces_paths();
    for g in member_globs {
        if g.contains('*') {
            if let Some(expanded) = expand_glob(root, &g) {
                for m_rel in expanded {
                    packages.push(parse_member(root, &m_rel)?);
                }
            }
        } else {
            packages.push(parse_member(root, Path::new(&g))?);
        }
    }

    let lockfile = read_lockfile(root)?;

    let workspace = Workspace {
        root: root.to_path_buf(),
        members: packages
            .iter()
            .filter(|p| p.name != root_name || packages.len() == 1)
            .map(|p| PathBuf::from(p.name.clone()))
            .collect(),
        adapter: "npm".to_string(),
        shared_metadata: IndexMap::new(),
    };
    Ok(Manifest::new(root.to_path_buf(), workspace, packages, lockfile))
}

fn parse_member(root: &Path, member: &Path) -> Result<Package> {
    let path = root.join(member).join("package.json");
    let raw = read_package_json(&path).map_err(|e| match e {
        NpmError::Io { .. } => NpmError::MissingWorkspaceMember {
            root: root.to_path_buf(),
            member: member.display().to_string(),
        },
        other => other,
    })?;
    convert_package(&raw, &path)
}

fn read_package_json(path: &Path) -> Result<raw::PackageJson> {
    let text = std::fs::read_to_string(path).map_err(|source| NpmError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    serde_json::from_str(&text).map_err(|source| NpmError::Json {
        path: path.to_path_buf(),
        source,
    })
}

fn read_lockfile(root: &Path) -> Result<Option<Lockfile>> {
    // Prefer pnpm-lock.yaml when present (richer + deterministic),
    // then package-lock.json (npm v2/v3 shape).
    let pnpm = root.join("pnpm-lock.yaml");
    if pnpm.exists() {
        let text = std::fs::read_to_string(&pnpm).map_err(|source| NpmError::Io {
            path: pnpm.clone(),
            source,
        })?;
        let parsed: raw::PnpmLock = serde_yaml::from_str(&text).map_err(|source| NpmError::Yaml {
            path: pnpm.clone(),
            source,
        })?;
        return Ok(Some(convert_pnpm_lock(&parsed)));
    }
    let npm = root.join("package-lock.json");
    if npm.exists() {
        let text = std::fs::read_to_string(&npm).map_err(|source| NpmError::Io {
            path: npm.clone(),
            source,
        })?;
        let parsed: raw::PackageLock = serde_json::from_str(&text).map_err(|source| NpmError::Json {
            path: npm.clone(),
            source,
        })?;
        return Ok(Some(convert_npm_lock(&parsed)));
    }
    Ok(None)
}

fn convert_package(p: &raw::PackageJson, manifest_path: &Path) -> Result<Package> {
    let name = p.name.clone().unwrap_or_else(|| "<unnamed>".to_string());
    let version_str = p.version.clone().unwrap_or_else(|| "0.0.0".to_string());
    let version = Version::parse(&pad_npm_version(&version_str)).ok_or_else(|| {
        NpmError::BadVersion {
            raw: version_str.clone(),
            context: format!("package `{name}` version"),
        }
    })?;

    let mut deps = Vec::new();
    for (n, c) in &p.dependencies {
        deps.push(npm_dep(n, c, DependencyKind::Direct)?);
    }
    for (n, c) in &p.dev_dependencies {
        deps.push(npm_dep(n, c, DependencyKind::Dev)?);
    }
    for (n, c) in &p.peer_dependencies {
        deps.push(npm_dep(n, c, DependencyKind::Peer)?);
    }
    for (n, c) in &p.optional_dependencies {
        deps.push(npm_dep(n, c, DependencyKind::Optional)?);
    }

    let manifest_dir = manifest_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();

    Ok(Package {
        name,
        version,
        source: PackageSource::Path {
            path: manifest_dir.display().to_string(),
        },
        registry: Registry::Npm,
        dependencies: deps,
        features: Vec::<Feature>::new(),
        build_steps: Vec::<BuildStep>::new(),
        license: p.license.clone(),
        description: p.description.clone(),
        authors: p.author_strings(),
        homepage: p.homepage.clone(),
        repository: p.repository_url(),
    })
}

fn npm_dep(name: &str, constraint: &str, kind: DependencyKind) -> Result<Dependency> {
    Ok(Dependency {
        name: name.to_string(),
        constraint: parse_npm_constraint(name, constraint)?,
        kind,
        features_enabled: Vec::new(),
        default_features: true,
        target_predicate: None,
        source_override: dep_source_override(constraint),
    })
}

fn dep_source_override(c: &str) -> Option<PackageSource> {
    // npm-style git + file specs in the version field.
    if let Some(rest) = c.strip_prefix("git+") {
        return Some(PackageSource::Git {
            url: rest.to_string(),
            rev: String::new(),
            subdir: None,
        });
    }
    if let Some(rest) = c.strip_prefix("file:") {
        return Some(PackageSource::Path {
            path: rest.to_string(),
        });
    }
    None
}

/// Parse an npm semver constraint into a [`VersionConstraint`].
/// Handles: `^X.Y.Z`, `~X.Y.Z`, `>=X.Y.Z`, `=X.Y.Z`, `X.Y.Z`,
/// `X.Y`, `X`, `*`, `latest`, `next`, `>=X <Y`, `git+...`, `file:...`.
fn parse_npm_constraint(name: &str, raw: &str) -> Result<VersionConstraint> {
    let raw = raw.trim();
    // git/file/path/url specs collapse to Any; the source_override
    // carries the resolution.
    if raw.starts_with("git+")
        || raw.starts_with("git:")
        || raw.starts_with("file:")
        || raw.starts_with("http://")
        || raw.starts_with("https://")
        || raw.starts_with("github:")
        || raw.starts_with("npm:")
    {
        return Ok(VersionConstraint {
            spec: ConstraintSpec::Any,
            native_syntax: Some(raw.to_string()),
        });
    }
    if raw.is_empty() || raw == "*" || raw == "latest" || raw == "next" {
        return Ok(VersionConstraint {
            spec: ConstraintSpec::Any,
            native_syntax: Some(raw.to_string()),
        });
    }
    let parts: Vec<&str> = raw.split_whitespace().collect();
    let spec = if parts.len() == 2 {
        let lo = parse_npm_one(name, parts[0])?;
        let hi = parse_npm_one(name, parts[1])?;
        if let (
            ConstraintSpec::GreaterEqual(a),
            ConstraintSpec::Less(b),
        ) = (&lo, &hi)
        {
            ConstraintSpec::Range {
                lower_inclusive: a.clone(),
                upper_exclusive: b.clone(),
            }
        } else {
            lo
        }
    } else {
        parse_npm_one(name, parts[0])?
    };
    Ok(VersionConstraint {
        spec,
        native_syntax: Some(raw.to_string()),
    })
}

fn parse_npm_one(name: &str, raw: &str) -> Result<ConstraintSpec> {
    let parse = |s: &str| {
        Version::parse(&pad_npm_version(s)).ok_or_else(|| NpmError::BadVersionReq {
            name: name.to_string(),
            raw: raw.to_string(),
        })
    };
    if let Some(rest) = raw.strip_prefix(">=") {
        Ok(ConstraintSpec::GreaterEqual(parse(rest.trim())?))
    } else if let Some(rest) = raw.strip_prefix("<=") {
        Ok(ConstraintSpec::LessEqual(parse(rest.trim())?))
    } else if let Some(rest) = raw.strip_prefix('>') {
        Ok(ConstraintSpec::Greater(parse(rest.trim())?))
    } else if let Some(rest) = raw.strip_prefix('<') {
        Ok(ConstraintSpec::Less(parse(rest.trim())?))
    } else if let Some(rest) = raw.strip_prefix('~') {
        Ok(ConstraintSpec::Tilde(parse(rest.trim())?))
    } else if let Some(rest) = raw.strip_prefix('^') {
        Ok(ConstraintSpec::Caret(parse(rest.trim())?))
    } else if let Some(rest) = raw.strip_prefix('=') {
        Ok(ConstraintSpec::Exact(parse(rest.trim())?))
    } else if raw == "*" || raw.is_empty() {
        Ok(ConstraintSpec::Any)
    } else {
        Ok(ConstraintSpec::Exact(parse(raw)?))
    }
}

/// npm accepts `1` / `1.2` / `1.2.3`; pad short forms with zeros and
/// strip non-semver suffixes (`-alpha.1` we keep; `-foo+meta` we keep
/// too — gen-types' Version supports pre + build).
fn pad_npm_version(raw: &str) -> String {
    let (core, suffix) = match raw.find(|c| c == '-' || c == '+') {
        Some(i) => (&raw[..i], &raw[i..]),
        None => (raw, ""),
    };
    let parts: Vec<&str> = core.split('.').collect();
    match parts.len() {
        0 => "0.0.0".to_string(),
        1 => format!("{}.0.0{}", parts[0], suffix),
        2 => format!("{}.{}.0{}", parts[0], parts[1], suffix),
        _ => raw.to_string(),
    }
}

fn convert_npm_lock(raw: &raw::PackageLock) -> Lockfile {
    let mut resolved: IndexMap<String, ResolvedPackage> = IndexMap::new();
    for (path_key, p) in &raw.packages {
        // npm v3 puts the root at "" + every dep at "node_modules/<name>".
        // We key by name/version so consumers can look up uniformly.
        let name = path_key
            .rsplit_once("node_modules/")
            .map(|(_, n)| n.to_string())
            .or_else(|| p.name.clone())
            .unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        let Some(version_raw) = &p.version else {
            continue;
        };
        let Some(version) = Version::parse(&pad_npm_version(version_raw)) else {
            continue;
        };
        let id = PackageId {
            name: name.clone(),
            version: version.clone(),
            registry: Registry::Npm,
        };
        let source = if p.resolved.as_deref().map(|s| s.starts_with("git+")).unwrap_or(false) {
            PackageSource::Git {
                url: p.resolved.clone().unwrap_or_default(),
                rev: String::new(),
                subdir: None,
            }
        } else {
            PackageSource::Registry {
                registry: Registry::Npm,
                registry_name: name.clone(),
                integrity_hash: p.integrity.clone(),
            }
        };
        resolved.insert(
            format!("{name}/{version_raw}"),
            ResolvedPackage {
                id,
                source,
                integrity: p.integrity.clone(),
                resolved_dependencies: Vec::new(),
            },
        );
    }
    let hash = ContentHash::of(
        &serde_json::to_vec(&resolved).unwrap_or_default(),
    );
    Lockfile {
        resolved,
        content_addressed_hash: hash,
    }
}

fn convert_pnpm_lock(raw: &raw::PnpmLock) -> Lockfile {
    let mut resolved: IndexMap<String, ResolvedPackage> = IndexMap::new();
    for (key, p) in &raw.packages {
        // pnpm keys look like "/<name>@<version>(qualifier)"; strip the
        // leading slash and qualifier.
        let stripped = key.trim_start_matches('/');
        let core = stripped.split('(').next().unwrap_or(stripped);
        let (name, version_raw) = match core.rsplit_once('@') {
            Some((n, v)) => (n.to_string(), v.to_string()),
            None => continue,
        };
        let Some(version) = Version::parse(&pad_npm_version(&version_raw)) else {
            continue;
        };
        let id = PackageId {
            name: name.clone(),
            version: version.clone(),
            registry: Registry::Npm,
        };
        let source = PackageSource::Registry {
            registry: Registry::Npm,
            registry_name: name.clone(),
            integrity_hash: p.resolution.as_ref().and_then(|r| r.integrity.clone()),
        };
        resolved.insert(
            format!("{name}/{version_raw}"),
            ResolvedPackage {
                id,
                source,
                integrity: p.resolution.as_ref().and_then(|r| r.integrity.clone()),
                resolved_dependencies: Vec::new(),
            },
        );
    }
    let hash = ContentHash::of(
        &serde_json::to_vec(&resolved).unwrap_or_default(),
    );
    Lockfile {
        resolved,
        content_addressed_hash: hash,
    }
}

fn expand_glob(root: &Path, pattern: &str) -> Option<Vec<PathBuf>> {
    let (prefix, rest) = pattern.split_once('*')?;
    if !rest.is_empty() && rest != "/" {
        return None;
    }
    let dir = root.join(prefix.trim_end_matches('/'));
    let mut out = Vec::new();
    let entries = std::fs::read_dir(&dir).ok()?;
    for e in entries.flatten() {
        let path = e.path();
        if path.is_dir() && path.join("package.json").exists() {
            if let Ok(rel) = path.strip_prefix(root) {
                out.push(rel.to_path_buf());
            }
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

    fn tempfile_dir() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let base = std::env::temp_dir().join(format!(
            "gen-npm-test-{}-{}-{:?}",
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

    #[test]
    fn parses_single_package() {
        let dir = tempfile_dir();
        write(
            &dir.join("package.json"),
            r#"{
              "name": "demo",
              "version": "1.2.3",
              "description": "a demo",
              "license": "MIT",
              "author": "acme <a@x>",
              "homepage": "https://example.org",
              "repository": { "url": "https://github.com/x/y" },
              "dependencies": { "lodash": "^4.17.0", "react": "18" },
              "devDependencies": { "jest": "~29.0" },
              "peerDependencies": { "react": ">=17" },
              "optionalDependencies": { "fsevents": "^2" }
            }"#,
        );
        let m = parse(&dir).unwrap();
        let p = m.find_package("demo").unwrap();
        assert_eq!(p.version, Version::new(1, 2, 3));
        assert_eq!(p.license.as_deref(), Some("MIT"));
        assert_eq!(p.description.as_deref(), Some("a demo"));
        assert_eq!(p.authors, vec!["acme <a@x>".to_string()]);
        assert_eq!(p.homepage.as_deref(), Some("https://example.org"));
        assert_eq!(p.repository.as_deref(), Some("https://github.com/x/y"));
        assert_eq!(p.dependencies.len(), 5);
        let lodash = p.dependencies.iter().find(|d| d.name == "lodash").unwrap();
        assert!(matches!(lodash.constraint.spec, ConstraintSpec::Caret(_)));
        let react = p
            .dependencies
            .iter()
            .find(|d| d.name == "react" && matches!(d.kind, DependencyKind::Direct))
            .unwrap();
        assert!(matches!(react.constraint.spec, ConstraintSpec::Exact(_)));
        let peer = p
            .dependencies
            .iter()
            .find(|d| d.name == "react" && matches!(d.kind, DependencyKind::Peer))
            .unwrap();
        assert!(matches!(peer.constraint.spec, ConstraintSpec::GreaterEqual(_)));
        let opt = p.dependencies.iter().find(|d| d.name == "fsevents").unwrap();
        assert!(matches!(opt.kind, DependencyKind::Optional));
    }

    #[test]
    fn parses_git_dep_with_source_override() {
        let dir = tempfile_dir();
        write(
            &dir.join("package.json"),
            r#"{
              "name": "g",
              "version": "0.0.1",
              "dependencies": { "fork": "git+https://github.com/x/y" }
            }"#,
        );
        let m = parse(&dir).unwrap();
        let p = m.find_package("g").unwrap();
        let fork = &p.dependencies[0];
        assert!(matches!(
            fork.source_override.as_ref().unwrap(),
            PackageSource::Git { .. }
        ));
    }

    #[test]
    fn parses_workspaces_field_with_glob() {
        let dir = tempfile_dir();
        write(
            &dir.join("package.json"),
            r#"{
              "name": "root",
              "version": "0.1.0",
              "workspaces": ["packages/*"]
            }"#,
        );
        write(
            &dir.join("packages/a/package.json"),
            r#"{ "name": "a", "version": "0.1.0" }"#,
        );
        write(
            &dir.join("packages/b/package.json"),
            r#"{ "name": "b", "version": "0.2.0" }"#,
        );
        let m = parse(&dir).unwrap();
        assert!(m.find_package("a").is_some());
        assert!(m.find_package("b").is_some());
    }

    #[test]
    fn parses_pnpm_lock_when_present() {
        let dir = tempfile_dir();
        write(
            &dir.join("package.json"),
            r#"{"name":"p","version":"0.1.0"}"#,
        );
        write(
            &dir.join("pnpm-lock.yaml"),
            r#"lockfileVersion: '6.0'
packages:
  /react@18.2.0:
    resolution:
      integrity: sha512-abcdef==
  /lodash@4.17.21:
    resolution:
      integrity: sha512-xyz==
"#,
        );
        let m = parse(&dir).unwrap();
        let lock = m.lockfile.as_ref().unwrap();
        assert!(lock.resolved.get("react/18.2.0").is_some());
        assert!(lock.resolved.get("lodash/4.17.21").is_some());
    }

    #[test]
    fn parses_package_lock_when_present() {
        let dir = tempfile_dir();
        write(
            &dir.join("package.json"),
            r#"{"name":"p","version":"0.1.0"}"#,
        );
        write(
            &dir.join("package-lock.json"),
            r#"{
              "name": "p",
              "lockfileVersion": 3,
              "packages": {
                "": { "name": "p", "version": "0.1.0" },
                "node_modules/react": { "version": "18.2.0", "integrity": "sha512-abc==" }
              }
            }"#,
        );
        let m = parse(&dir).unwrap();
        let lock = m.lockfile.as_ref().unwrap();
        assert!(lock.resolved.get("react/18.2.0").is_some());
    }

    #[test]
    fn parses_range_constraint() {
        let dir = tempfile_dir();
        write(
            &dir.join("package.json"),
            r#"{
              "name": "p",
              "version": "0.1.0",
              "dependencies": { "foo": ">=1.2.0 <2.0.0" }
            }"#,
        );
        let m = parse(&dir).unwrap();
        let p = m.find_package("p").unwrap();
        let foo = &p.dependencies[0];
        match &foo.constraint.spec {
            ConstraintSpec::Range {
                lower_inclusive,
                upper_exclusive,
            } => {
                assert_eq!(*lower_inclusive, Version::new(1, 2, 0));
                assert_eq!(*upper_exclusive, Version::new(2, 0, 0));
            }
            other => panic!("expected Range, got {other:?}"),
        }
    }

    #[test]
    fn parses_short_version_strings() {
        let dir = tempfile_dir();
        write(
            &dir.join("package.json"),
            r#"{
              "name": "p",
              "version": "1",
              "dependencies": { "x": "2.0" }
            }"#,
        );
        let m = parse(&dir).unwrap();
        let p = m.find_package("p").unwrap();
        assert_eq!(p.version, Version::new(1, 0, 0));
        assert!(matches!(p.dependencies[0].constraint.spec, ConstraintSpec::Exact(_)));
    }

    #[test]
    fn missing_workspace_member_surfaces_typed_error() {
        let dir = tempfile_dir();
        write(
            &dir.join("package.json"),
            r#"{
              "name": "p",
              "version": "0.1.0",
              "workspaces": ["packages/missing"]
            }"#,
        );
        let e = parse(&dir).unwrap_err();
        assert!(matches!(e, NpmError::MissingWorkspaceMember { .. }));
    }
}
