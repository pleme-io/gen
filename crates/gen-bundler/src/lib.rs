//! `gen-bundler` — Ruby/Bundler adapter for the `gen` ecosystem.
//!
//! Reads a directory containing `Gemfile` + (optionally) `Gemfile.lock`
//! and emits a typed [`gen_types::Manifest`]. Gemfile parsing is
//! line-oriented + handles the typical `gem 'name', '~> 1.2'`,
//! `gem 'name', git: '…'`, `group :development do ... end`, and
//! `source 'https://rubygems.org'` shapes. Full Ruby-DSL eval is
//! intentionally deferred — pleme-io's Bundler usage is the typical
//! declarative shape this parser covers.

pub mod adapter;
pub mod build_spec;
pub mod error;
pub mod invariants;
pub mod quirks;

pub use adapter::{ctx_for, BundlerAdapter};
pub use error::{BundlerError, Result};

use std::path::{Path, PathBuf};

use indexmap::IndexMap;

use gen_types::{
    BuildStep, ConstraintSpec, ContentHash, Dependency, DependencyKind, Feature, Lockfile,
    Manifest, Package, PackageId, PackageSource, Registry, ResolvedPackage, Target,
    TargetPredicate, Version, VersionConstraint, Workspace,
};

/// Parse the Bundler project rooted at `root`.
pub fn parse(root: &Path) -> Result<Manifest> {
    let gemfile_path = root.join("Gemfile");
    let raw = std::fs::read_to_string(&gemfile_path).map_err(|source| BundlerError::Io {
        path: gemfile_path.clone(),
        source,
    })?;
    let parsed = parse_gemfile(&raw);

    let pkg = Package {
        name: root
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "<bundler-project>".to_string()),
        version: Version::new(0, 0, 0),
        source: PackageSource::Path {
            path: root.display().to_string(),
        },
        registry: Registry::RubyGems,
        dependencies: parsed.dependencies,
        features: Vec::<Feature>::new(),
        build_steps: Vec::<BuildStep>::new(),
        license: None,
        description: None,
        authors: Vec::new(),
        homepage: None,
        repository: None,
    };

    let lockfile = match std::fs::read_to_string(root.join("Gemfile.lock")) {
        Ok(s) => Some(parse_gemfile_lock(&s)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(source) => {
            return Err(BundlerError::Io {
                path: root.join("Gemfile.lock"),
                source,
            });
        }
    };

    let workspace = Workspace::single_package(root.to_path_buf(), "bundler");
    Ok(Manifest::new(
        root.to_path_buf(),
        workspace,
        vec![pkg],
        lockfile,
    ))
}

struct GemfileParse {
    dependencies: Vec<Dependency>,
    #[allow(dead_code)]
    sources: Vec<String>,
}

/// Line-oriented Gemfile parser. Handles `source`, `gem`, `group`
/// nesting (1 deep), and standard option flags (`git`, `branch`,
/// `tag`, `ref`, `path`, `require`, `platforms`, `optional`).
fn parse_gemfile(text: &str) -> GemfileParse {
    let mut deps: Vec<Dependency> = Vec::new();
    let mut sources: Vec<String> = Vec::new();
    let mut group_stack: Vec<Vec<String>> = Vec::new();
    let mut platform_stack: Vec<Vec<String>> = Vec::new();

    for raw_line in text.lines() {
        let line = strip_comment(raw_line).trim().to_string();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("source ") {
            sources.push(strip_quotes(rest.trim()).to_string());
            continue;
        }
        if let Some(rest) = line.strip_prefix("group ") {
            // group :dev, :test do
            let body = rest.trim_end_matches(" do").trim();
            let names: Vec<String> = body
                .split(',')
                .map(|s| s.trim().trim_start_matches(':').to_string())
                .filter(|s| !s.is_empty())
                .collect();
            group_stack.push(names);
            continue;
        }
        if let Some(rest) = line.strip_prefix("platforms ") {
            let body = rest.trim_end_matches(" do").trim();
            let names: Vec<String> = body
                .split(',')
                .map(|s| s.trim().trim_start_matches(':').to_string())
                .filter(|s| !s.is_empty())
                .collect();
            platform_stack.push(names);
            continue;
        }
        if line == "end" {
            if !platform_stack.is_empty() {
                platform_stack.pop();
            } else if !group_stack.is_empty() {
                group_stack.pop();
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("gem ") {
            if let Some(dep) =
                parse_gem_line(rest, group_stack.last(), platform_stack.last())
            {
                deps.push(dep);
            }
            continue;
        }
    }

    GemfileParse {
        dependencies: deps,
        sources,
    }
}

fn parse_gem_line(
    rest: &str,
    groups: Option<&Vec<String>>,
    platforms: Option<&Vec<String>>,
) -> Option<Dependency> {
    // gem 'name', '~> 1.2', git: '…', branch: '…', require: false
    let parts: Vec<&str> = rest.split(',').map(str::trim).collect();
    if parts.is_empty() {
        return None;
    }
    let name = strip_quotes(parts[0]).to_string();
    if name.is_empty() {
        return None;
    }
    let mut version_str = None;
    let mut git_url = None;
    let mut git_rev = None;
    let mut path_override = None;
    let mut optional = false;
    let mut platform_opt = None;

    for part in &parts[1..] {
        if !part.contains(':') && (part.starts_with('\'') || part.starts_with('"')) {
            version_str = Some(strip_quotes(part).to_string());
            continue;
        }
        if let Some((k, v)) = part.split_once(':') {
            let key = k.trim();
            let val = strip_quotes(v.trim());
            match key {
                "git" => git_url = Some(val.to_string()),
                "branch" | "tag" | "ref" => {
                    git_rev = Some(val.to_string());
                }
                "path" => path_override = Some(val.to_string()),
                "optional" => optional = val == "true",
                "platforms" | "platform" => {
                    let names: Vec<String> = val
                        .split([',', ' '])
                        .map(|s| s.trim().trim_start_matches(':').to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                    if !names.is_empty() {
                        platform_opt = Some(names);
                    }
                }
                _ => {}
            }
        }
    }

    let kind = if groups
        .map(|g| g.iter().any(|s| s == "development" || s == "test"))
        .unwrap_or(false)
    {
        DependencyKind::Dev
    } else if optional {
        DependencyKind::Optional
    } else {
        DependencyKind::Direct
    };

    let constraint = match &version_str {
        Some(v) => parse_gemfile_constraint(&name, v).unwrap_or_else(|_| {
            VersionConstraint {
                spec: ConstraintSpec::Any,
                native_syntax: Some(v.clone()),
            }
        }),
        None => VersionConstraint::from_spec(ConstraintSpec::Any),
    };

    let source_override = if let Some(url) = &git_url {
        Some(PackageSource::Git {
            url: url.clone(),
            rev: git_rev.unwrap_or_default(),
            subdir: None,
        })
    } else {
        path_override.map(|p| PackageSource::Path { path: p })
    };

    let combined_platforms = platforms.cloned().or(platform_opt);
    let target_predicate = combined_platforms
        .filter(|p| !p.is_empty())
        .map(|items| TargetPredicate::BundlerPlatforms { items });

    Some(Dependency {
        name,
        constraint,
        kind,
        features_enabled: Vec::new(),
        default_features: true,
        target_predicate,
        source_override,
    })
}

fn parse_gemfile_constraint(name: &str, raw: &str) -> Result<VersionConstraint> {
    let raw = raw.trim();
    // Bundler accepts comma-separated multi-constraint: '>= 1.0', '< 2.0'
    let parts: Vec<&str> = raw.split(',').map(str::trim).collect();
    if parts.is_empty() {
        return Ok(VersionConstraint::from_spec(ConstraintSpec::Any));
    }
    let mut atoms = Vec::with_capacity(parts.len());
    for p in &parts {
        atoms.push(parse_gemfile_one(name, p)?);
    }
    let spec = if atoms.len() == 2 {
        if let (
            ConstraintSpec::GreaterEqual(lo),
            ConstraintSpec::Less(hi),
        ) = (&atoms[0], &atoms[1])
        {
            ConstraintSpec::Range {
                lower_inclusive: lo.clone(),
                upper_exclusive: hi.clone(),
            }
        } else {
            atoms[0].clone()
        }
    } else {
        atoms[0].clone()
    };
    Ok(VersionConstraint {
        spec,
        native_syntax: Some(raw.to_string()),
    })
}

fn parse_gemfile_one(name: &str, raw: &str) -> Result<ConstraintSpec> {
    let parse = |s: &str| {
        Version::parse(&pad_gem_version(s)).ok_or_else(|| BundlerError::BadVersionReq {
            name: name.to_string(),
            raw: raw.to_string(),
        })
    };
    if let Some(rest) = raw.strip_prefix("~>") {
        // ~> is Bundler's pessimistic operator — closest analog is Tilde.
        Ok(ConstraintSpec::Tilde(parse(rest.trim())?))
    } else if let Some(rest) = raw.strip_prefix(">=") {
        Ok(ConstraintSpec::GreaterEqual(parse(rest.trim())?))
    } else if let Some(rest) = raw.strip_prefix("<=") {
        Ok(ConstraintSpec::LessEqual(parse(rest.trim())?))
    } else if let Some(rest) = raw.strip_prefix('>') {
        Ok(ConstraintSpec::Greater(parse(rest.trim())?))
    } else if let Some(rest) = raw.strip_prefix('<') {
        Ok(ConstraintSpec::Less(parse(rest.trim())?))
    } else if let Some(rest) = raw.strip_prefix('=') {
        Ok(ConstraintSpec::Exact(parse(rest.trim())?))
    } else {
        Ok(ConstraintSpec::Exact(parse(raw.trim())?))
    }
}

/// Gemfile gems accept `1`, `1.2`, `1.2.3`, `1.2.3.4` (RubyGems' 4th
/// segment). Pad short forms; Version::parse handles the 4-segment
/// case via its `extension` field.
fn pad_gem_version(raw: &str) -> String {
    let core = raw.split('-').next().unwrap_or(raw);
    let parts: Vec<&str> = core.split('.').collect();
    match parts.len() {
        0 => "0.0.0".to_string(),
        1 => format!("{}.0.0", parts[0]),
        2 => format!("{}.{}.0", parts[0], parts[1]),
        _ => raw.to_string(),
    }
}

fn strip_comment(s: &str) -> &str {
    match s.find('#') {
        Some(i) => &s[..i],
        None => s,
    }
}

fn strip_quotes(s: &str) -> &str {
    let s = s.trim();
    s.trim_start_matches('\'')
        .trim_end_matches('\'')
        .trim_start_matches('"')
        .trim_end_matches('"')
}

/// Convenience evaluator — confirms which deps are active for a
/// given target. Engines use this to filter dev/test deps off the
/// runtime closure.
#[must_use]
pub fn deps_for_target(deps: &[Dependency], target: &Target) -> Vec<usize> {
    deps.iter()
        .enumerate()
        .filter_map(|(i, d)| match &d.target_predicate {
            None => Some(i),
            Some(p) => {
                if p.matches(target) {
                    Some(i)
                } else {
                    None
                }
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(path: &PathBuf, body: &str) {
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
            "gen-bundler-test-{}-{}-{:?}",
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
    fn parses_standard_gem_lines() {
        let dir = tempfile_dir();
        write(
            &dir.join("Gemfile"),
            r#"source 'https://rubygems.org'

gem 'rails', '~> 7.0'
gem 'pg', '>= 1.0'
gem 'puma'
gem 'sidekiq', '6.5.7'
"#,
        );
        let m = parse(&dir).unwrap();
        let p = &m.packages[0];
        assert_eq!(p.dependencies.len(), 4);
        let rails = p.dependencies.iter().find(|d| d.name == "rails").unwrap();
        assert!(matches!(rails.constraint.spec, ConstraintSpec::Tilde(_)));
        let pg = p.dependencies.iter().find(|d| d.name == "pg").unwrap();
        assert!(matches!(pg.constraint.spec, ConstraintSpec::GreaterEqual(_)));
        let puma = p.dependencies.iter().find(|d| d.name == "puma").unwrap();
        assert!(matches!(puma.constraint.spec, ConstraintSpec::Any));
        let sk = p.dependencies.iter().find(|d| d.name == "sidekiq").unwrap();
        assert!(matches!(sk.constraint.spec, ConstraintSpec::Exact(_)));
    }

    #[test]
    fn group_dev_marks_deps_as_dev() {
        let dir = tempfile_dir();
        write(
            &dir.join("Gemfile"),
            r#"gem 'a'
group :development, :test do
  gem 'rspec'
  gem 'pry'
end
gem 'b'
"#,
        );
        let m = parse(&dir).unwrap();
        let p = &m.packages[0];
        let rspec = p.dependencies.iter().find(|d| d.name == "rspec").unwrap();
        assert!(matches!(rspec.kind, DependencyKind::Dev));
        let a = p.dependencies.iter().find(|d| d.name == "a").unwrap();
        assert!(matches!(a.kind, DependencyKind::Direct));
    }

    #[test]
    fn parses_git_source_override() {
        let dir = tempfile_dir();
        write(
            &dir.join("Gemfile"),
            r#"gem 'forked', git: 'https://github.com/x/y', branch: 'main'
"#,
        );
        let m = parse(&dir).unwrap();
        let p = &m.packages[0];
        let f = &p.dependencies[0];
        match f.source_override.as_ref().unwrap() {
            PackageSource::Git { url, rev, .. } => {
                assert_eq!(url, "https://github.com/x/y");
                assert_eq!(rev, "main");
            }
            other => panic!("expected Git, got {other:?}"),
        }
    }

    #[test]
    fn platforms_block_attaches_target_predicate() {
        let dir = tempfile_dir();
        write(
            &dir.join("Gemfile"),
            r#"platforms :ruby, :mri do
  gem 'native-thing'
end
"#,
        );
        let m = parse(&dir).unwrap();
        let p = &m.packages[0];
        let d = &p.dependencies[0];
        assert!(matches!(
            d.target_predicate.as_ref().unwrap(),
            TargetPredicate::BundlerPlatforms { items } if items == &vec!["ruby".to_string(), "mri".to_string()]
        ));
    }

    #[test]
    fn parses_gemfile_lock_when_present() {
        let dir = tempfile_dir();
        write(&dir.join("Gemfile"), "gem 'rails'\n");
        write(
            &dir.join("Gemfile.lock"),
            r#"GEM
  remote: https://rubygems.org/
  specs:
    rails (7.1.3)
      activesupport (>= 7.1.0)
    activesupport (7.1.3)

PLATFORMS
  ruby

DEPENDENCIES
  rails

BUNDLED WITH
   2.4.10
"#,
        );
        let m = parse(&dir).unwrap();
        let lock = m.lockfile.as_ref().unwrap();
        assert!(lock.resolved.get("rails/7.1.3").is_some());
        assert!(lock.resolved.get("activesupport/7.1.3").is_some());
    }
}

/// Parse a Gemfile.lock into the typed [`Lockfile`] shape. Line-
/// oriented walk over the `GEM` / `specs:` section; sub-dependency
/// edges are intentionally not yet materialized (Bundler resolves
/// them at lock time; sub-edges are M1+ enrichment).
pub fn parse_gemfile_lock(text: &str) -> Lockfile {
    let mut resolved: IndexMap<String, ResolvedPackage> = IndexMap::new();
    let mut in_specs = false;
    for raw in text.lines() {
        let line = raw.trim_end();
        if line.is_empty() {
            continue;
        }
        if line == "  specs:" {
            in_specs = true;
            continue;
        }
        if !line.starts_with(' ') {
            in_specs = false;
            continue;
        }
        if !in_specs {
            continue;
        }
        // Spec entries: "    name (1.2.3)" — exactly 4 leading spaces.
        // Sub-deps: "      name (>= 1.0)" — 6 spaces; skip.
        if line.starts_with("    ") && !line.starts_with("      ") {
            let body = line.trim();
            if let Some((name, ver_paren)) = body.rsplit_once(' ') {
                let v = ver_paren.trim_start_matches('(').trim_end_matches(')');
                if let Some(version) = Version::parse(&pad_gem_version(v)) {
                    let id = PackageId {
                        name: name.to_string(),
                        version: version.clone(),
                        registry: Registry::RubyGems,
                    };
                    resolved.insert(
                        format!("{name}/{v}"),
                        ResolvedPackage {
                            id,
                            source: PackageSource::Registry {
                                registry: Registry::RubyGems,
                                registry_name: name.to_string(),
                                integrity_hash: None,
                            },
                            integrity: None,
                            resolved_dependencies: Vec::new(),
                        },
                    );
                }
            }
        }
    }
    let hash = ContentHash::of(&serde_json::to_vec(&resolved).unwrap_or_default());
    Lockfile {
        resolved,
        content_addressed_hash: hash,
    }
}
