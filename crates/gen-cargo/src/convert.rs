//! Conversion: raw cargo TOML/lock shapes → typed `gen_types` IR.
//! Keeps the deserialization layer in [`crate::raw`] dumb and pushes
//! every Cargo-ism (workspace inheritance, dep-source shape, version
//! quirks) here. New Cargo features land here, not in `raw.rs`.

use std::path::{Path, PathBuf};

use gen_types::{
    BuildStep, ConstraintSpec, ContentHash, Dependency, DependencyKind, Feature, FeatureRef,
    Lockfile, Manifest, Package, PackageId, PackageSource, Registry, ResolvedPackage,
    TargetPredicate, Version, VersionConstraint, Workspace,
};
use indexmap::IndexMap;

use crate::error::{CargoError, Result};
use crate::raw::{
    CargoLock, CargoToml, RawDep, RawDepDetail, RawInheritedString, RawInheritedStringList,
    RawLockPackage, RawPackage,
};

/// Convert a parsed [`CargoToml`] + (optional) workspace-level package
/// metadata into a typed [`Package`]. Used both for single-crate
/// repos and for each workspace member.
pub fn convert_package(
    raw: &CargoToml,
    manifest_path: &Path,
    workspace_pkg: Option<&RawPackage>,
    workspace_deps: &IndexMap<String, RawDep>,
) -> Result<Package> {
    let pkg = raw
        .package
        .as_ref()
        .ok_or_else(|| CargoError::EmptyManifest {
            path: manifest_path.to_path_buf(),
        })?;

    let ws_name = workspace_pkg.and_then(|w| w.name.as_ref().and_then(literal));
    let name = pkg
        .name
        .as_ref()
        .and_then(|n| n.resolve(ws_name))
        .ok_or_else(|| CargoError::EmptyManifest {
            path: manifest_path.to_path_buf(),
        })?
        .to_string();

    let ws_version = workspace_pkg.and_then(|w| w.version.as_ref().and_then(literal));
    let version_raw = pkg
        .version
        .as_ref()
        .and_then(|v| v.resolve(ws_version))
        .unwrap_or("0.0.0");
    let version = Version::parse(version_raw).ok_or_else(|| CargoError::BadVersion {
        raw: version_raw.to_string(),
        context: format!("package `{name}` version"),
    })?;

    let description = resolve_str(
        pkg.description.as_ref(),
        workspace_pkg.and_then(|w| w.description.as_ref()),
    )
    .map(str::to_string);
    let license = resolve_str(
        pkg.license.as_ref(),
        workspace_pkg.and_then(|w| w.license.as_ref()),
    )
    .map(str::to_string);
    let repository = resolve_str(
        pkg.repository.as_ref(),
        workspace_pkg.and_then(|w| w.repository.as_ref()),
    )
    .map(str::to_string);
    let homepage = resolve_str(
        pkg.homepage.as_ref(),
        workspace_pkg.and_then(|w| w.homepage.as_ref()),
    )
    .map(str::to_string);
    let authors = resolve_strs(
        pkg.authors.as_ref(),
        workspace_pkg.and_then(|w| w.authors.as_ref()),
    )
    .map(<[String]>::to_vec)
    .unwrap_or_default();

    // Direct + dev + build dependencies (top-level), then targeted.
    let mut dependencies = Vec::new();
    for (n, d) in &raw.dependencies {
        dependencies.push(convert_dep(n, d, DependencyKind::Direct, None, workspace_deps, manifest_path)?);
    }
    for (n, d) in &raw.dev_dependencies {
        dependencies.push(convert_dep(n, d, DependencyKind::Dev, None, workspace_deps, manifest_path)?);
    }
    for (n, d) in &raw.build_dependencies {
        dependencies.push(convert_dep(n, d, DependencyKind::Build, None, workspace_deps, manifest_path)?);
    }
    for (cfg, block) in &raw.target {
        let predicate = parse_target_key(cfg);
        for (n, d) in &block.dependencies {
            dependencies.push(convert_dep(
                n,
                d,
                DependencyKind::Direct,
                Some(predicate.clone()),
                workspace_deps,
                manifest_path,
            )?);
        }
        for (n, d) in &block.dev_dependencies {
            dependencies.push(convert_dep(
                n,
                d,
                DependencyKind::Dev,
                Some(predicate.clone()),
                workspace_deps,
                manifest_path,
            )?);
        }
        for (n, d) in &block.build_dependencies {
            dependencies.push(convert_dep(
                n,
                d,
                DependencyKind::Build,
                Some(predicate.clone()),
                workspace_deps,
                manifest_path,
            )?);
        }
    }

    let features: Vec<Feature> = raw
        .features
        .iter()
        .map(|(name, implies)| Feature {
            name: name.clone(),
            implies: implies.iter().map(|s| FeatureRef::parse(s)).collect(),
        })
        .collect();

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
        registry: Registry::CratesIo,
        dependencies,
        features,
        build_steps: Vec::<BuildStep>::new(),
        license,
        description,
        authors,
        homepage,
        repository,
    })
}

fn literal(s: &RawInheritedString) -> Option<&str> {
    match s {
        RawInheritedString::Literal(v) => Some(v.as_str()),
        RawInheritedString::Inherit { .. } => None,
    }
}

fn literal_list(s: &RawInheritedStringList) -> Option<&[String]> {
    match s {
        RawInheritedStringList::Literal(v) => Some(v.as_slice()),
        RawInheritedStringList::Inherit { .. } => None,
    }
}

fn resolve_str<'a>(
    field: Option<&'a RawInheritedString>,
    workspace: Option<&'a RawInheritedString>,
) -> Option<&'a str> {
    let ws = workspace.and_then(literal);
    field.and_then(|f| f.resolve(ws))
}

fn resolve_strs<'a>(
    field: Option<&'a RawInheritedStringList>,
    workspace: Option<&'a RawInheritedStringList>,
) -> Option<&'a [String]> {
    let ws = workspace.and_then(literal_list);
    field.and_then(|f| f.resolve(ws))
}

/// Translate `target.'cfg(unix)'` / `target.'x86_64-pc-windows-msvc'`
/// TOML keys to a [`TargetPredicate`].
fn parse_target_key(key: &str) -> TargetPredicate {
    if key.starts_with("cfg(") {
        TargetPredicate::CargoCfg(key.to_string())
    } else {
        // Explicit target triple; encode as cfg(target = "...") so the
        // engine sees it as a typed predicate and not a stringly key.
        TargetPredicate::CargoCfg(format!("cfg(target = \"{key}\")"))
    }
}

/// Convert one Cargo dep specification to a typed [`Dependency`].
/// Resolves `{ workspace = true }` against the workspace-level
/// dependency table.
fn convert_dep(
    name: &str,
    dep: &RawDep,
    kind: DependencyKind,
    target_predicate: Option<TargetPredicate>,
    workspace_deps: &IndexMap<String, RawDep>,
    manifest_path: &Path,
) -> Result<Dependency> {
    let detail = match dep {
        RawDep::Short(v) => RawDepDetail {
            version: Some(v.clone()),
            ..Default::default()
        },
        RawDep::Long(d) => {
            if d.workspace {
                // Resolve from workspace.dependencies; fall back to an
                // empty detail if unresolvable so the engine sees an
                // explicit no-source dep + can surface the error.
                let ws = workspace_deps.get(name);
                let mut base = match ws {
                    Some(RawDep::Short(v)) => RawDepDetail {
                        version: Some(v.clone()),
                        ..Default::default()
                    },
                    Some(RawDep::Long(w)) => w.clone(),
                    None => RawDepDetail::default(),
                };
                base.features.extend_from_slice(&d.features);
                if d.optional {
                    base.optional = true;
                }
                if !d.default_features {
                    base.default_features = false;
                }
                base
            } else {
                d.clone()
            }
        }
    };

    let kind = if detail.optional && matches!(kind, DependencyKind::Direct) {
        DependencyKind::Optional
    } else {
        kind
    };

    let constraint = parse_version_constraint(name, detail.version.as_deref())?;

    let source_override = if let Some(git) = &detail.git {
        let rev = detail
            .rev
            .clone()
            .or_else(|| detail.tag.clone())
            .or_else(|| detail.branch.clone())
            .unwrap_or_default();
        Some(PackageSource::Git {
            url: git.clone(),
            rev,
            subdir: None,
        })
    } else if let Some(path) = &detail.path {
        let abs = manifest_path
            .parent()
            .map(|p| p.join(path))
            .unwrap_or_else(|| PathBuf::from(path));
        Some(PackageSource::Path {
            path: abs.display().to_string(),
        })
    } else {
        None
    };

    Ok(Dependency {
        name: detail
            .package
            .clone()
            .unwrap_or_else(|| name.to_string()),
        constraint,
        kind,
        features_enabled: detail.features.clone(),
        default_features: detail.default_features,
        target_predicate,
        source_override,
    })
}

/// Parse a Cargo version requirement (`"1.0"`, `"=1.0.1"`, `">=1, <2"`)
/// into a [`VersionConstraint`]. Cargo's default operator is caret, so
/// `"1.0"` means `^1.0`. Multi-constraint requirements (comma-separated)
/// pick the strongest bound and stash the original syntax in
/// `native_syntax` for round-trip fidelity.
pub fn parse_version_constraint(name: &str, raw: Option<&str>) -> Result<VersionConstraint> {
    let Some(raw) = raw else {
        return Ok(VersionConstraint::from_spec(ConstraintSpec::Any));
    };
    let parts: Vec<&str> = raw
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if parts.is_empty() {
        return Ok(VersionConstraint::from_spec(ConstraintSpec::Any));
    }
    let mut atoms = Vec::with_capacity(parts.len());
    for part in &parts {
        atoms.push(parse_one_spec(name, part)?);
    }
    // Two-part >=X,<Y → ConstraintSpec::Range; otherwise pick the first
    // atom + retain the original raw syntax in native_syntax so the
    // engine can round-trip back.
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

/// Parse a Cargo version requirement segment into a [`Version`]. Cargo
/// accepts `"1"` / `"1.0"` / `"1.0.0"`; missing segments are zero-padded
/// per the Cargo spec.
fn parse_cargo_version(raw: &str) -> Option<Version> {
    // Drop pre-release / build metadata before padding — they only attach
    // to the patch segment in SemVer, so we can re-attach after padding.
    let (core, suffix) = match raw.find(|c| c == '-' || c == '+') {
        Some(i) => (&raw[..i], &raw[i..]),
        None => (raw, ""),
    };
    let parts: Vec<&str> = core.split('.').collect();
    let padded = match parts.len() {
        0 => return None,
        1 => format!("{}.0.0{}", parts[0], suffix),
        2 => format!("{}.{}.0{}", parts[0], parts[1], suffix),
        _ => raw.to_string(),
    };
    Version::parse(&padded)
}

fn parse_one_spec(name: &str, raw: &str) -> Result<ConstraintSpec> {
    let parse = |s: &str, ctx: &str| -> Result<Version> {
        parse_cargo_version(s).ok_or_else(|| CargoError::BadVersionReq {
            name: name.to_string(),
            raw: ctx.to_string(),
        })
    };
    if let Some(rest) = raw.strip_prefix(">=") {
        Ok(ConstraintSpec::GreaterEqual(parse(rest.trim(), raw)?))
    } else if let Some(rest) = raw.strip_prefix("<=") {
        Ok(ConstraintSpec::LessEqual(parse(rest.trim(), raw)?))
    } else if let Some(rest) = raw.strip_prefix('>') {
        Ok(ConstraintSpec::Greater(parse(rest.trim(), raw)?))
    } else if let Some(rest) = raw.strip_prefix('<') {
        Ok(ConstraintSpec::Less(parse(rest.trim(), raw)?))
    } else if let Some(rest) = raw.strip_prefix('~') {
        Ok(ConstraintSpec::Tilde(parse(rest.trim(), raw)?))
    } else if let Some(rest) = raw.strip_prefix('^') {
        Ok(ConstraintSpec::Caret(parse(rest.trim(), raw)?))
    } else if let Some(rest) = raw.strip_prefix('=') {
        Ok(ConstraintSpec::Exact(parse(rest.trim(), raw)?))
    } else if raw == "*" {
        Ok(ConstraintSpec::Any)
    } else {
        // Cargo's bare-version default is caret semantics.
        Ok(ConstraintSpec::Caret(parse(raw, raw)?))
    }
}

/// Convert a Cargo.lock into the typed [`Lockfile`] shape. Source
/// strings are mapped to [`PackageSource`]; checksums (sha256 hex) are
/// stored as the integrity field.
pub fn convert_lockfile(raw: &CargoLock, lock_path: &Path) -> Result<Lockfile> {
    let mut resolved: IndexMap<String, ResolvedPackage> = IndexMap::with_capacity(raw.packages.len());
    // First pass: build PackageId for every entry so cross-package
    // resolved_dependencies can be looked up by name in the second pass.
    let mut ids: IndexMap<String, PackageId> = IndexMap::with_capacity(raw.packages.len());
    for p in &raw.packages {
        let version = Version::parse(&p.version).ok_or_else(|| CargoError::BadVersion {
            raw: p.version.clone(),
            context: format!("lock entry `{}`", p.name),
        })?;
        ids.insert(
            p.name.clone(),
            PackageId {
                name: p.name.clone(),
                version,
                registry: registry_for(p),
            },
        );
    }

    for p in &raw.packages {
        let id = ids
            .get(&p.name)
            .cloned()
            .ok_or(CargoError::LockfileMissingField {
                path: lock_path.to_path_buf(),
                entry: p.name.clone(),
                field: "name",
            })?;
        let source = parse_lock_source(p);
        let integrity = p.checksum.clone().map(|h| format!("sha256:{h}"));
        let resolved_dependencies = p
            .dependencies
            .iter()
            .filter_map(|d| {
                let name = d.split_whitespace().next().unwrap_or(d);
                ids.get(name).cloned()
            })
            .collect();
        let key = format!("{}/{}", p.name, p.version);
        resolved.insert(
            key,
            ResolvedPackage {
                id,
                source,
                integrity,
                resolved_dependencies,
            },
        );
    }

    let content_addressed_hash = compute_lockfile_hash(&resolved);
    Ok(Lockfile {
        resolved,
        content_addressed_hash,
    })
}

fn compute_lockfile_hash(resolved: &IndexMap<String, ResolvedPackage>) -> ContentHash {
    // Hash the canonical JSON of the resolved map. This is deterministic
    // because IndexMap preserves insertion order + serde_json is stable.
    let bytes = serde_json::to_vec(resolved).unwrap_or_default();
    ContentHash::of(&bytes)
}

fn registry_for(p: &RawLockPackage) -> Registry {
    match p.source.as_deref() {
        Some(s) if s.starts_with("registry+https://github.com/rust-lang/crates.io-index") => {
            Registry::CratesIo
        }
        Some(s) if s.starts_with("git+") => Registry::None,
        Some(s) if s.starts_with("registry+") => {
            let url = s.trim_start_matches("registry+").to_string();
            Registry::Private {
                url,
                protocol: "sparse".to_string(),
            }
        }
        _ => Registry::None,
    }
}

fn parse_lock_source(p: &RawLockPackage) -> PackageSource {
    match p.source.as_deref() {
        Some(s) if s.starts_with("registry+") => PackageSource::Registry {
            registry: registry_for(p),
            registry_name: p.name.clone(),
            integrity_hash: p.checksum.clone().map(|h| format!("sha256:{h}")),
        },
        Some(s) if s.starts_with("git+") => {
            let trimmed = s.trim_start_matches("git+");
            let (url, rev) = trimmed
                .rsplit_once('#')
                .map(|(u, f)| (u.to_string(), f.to_string()))
                .unwrap_or_else(|| (trimmed.to_string(), String::new()));
            PackageSource::Git {
                url,
                rev,
                subdir: None,
            }
        }
        _ => PackageSource::Path {
            path: String::new(),
        },
    }
}

/// Convert a parsed [`CargoToml`] workspace block to the typed
/// [`Workspace`] shape. Member paths are kept relative to the
/// workspace root so the engine can re-anchor on a different host.
pub fn convert_workspace(raw: &CargoToml, root: &Path) -> Workspace {
    if let Some(ws) = raw.workspace.as_ref() {
        let members = ws.members.iter().map(PathBuf::from).collect();
        let mut shared_metadata = IndexMap::new();
        if let Some(pkg) = &ws.package {
            if let Some(v) = pkg.version.as_ref().and_then(literal) {
                shared_metadata.insert("version".to_string(), v.to_string());
            }
            if let Some(v) = pkg.license.as_ref().and_then(literal) {
                shared_metadata.insert("license".to_string(), v.to_string());
            }
            if let Some(v) = pkg.edition.as_ref().and_then(literal) {
                shared_metadata.insert("edition".to_string(), v.to_string());
            }
            if let Some(v) = pkg.rust_version.as_ref().and_then(literal) {
                shared_metadata.insert("rust-version".to_string(), v.to_string());
            }
        }
        Workspace {
            root: root.to_path_buf(),
            members,
            adapter: "cargo".to_string(),
            shared_metadata,
        }
    } else {
        Workspace::single_package(root.to_path_buf(), "cargo")
    }
}

/// Top-level adapter entrypoint. Single-crate or workspace root.
pub fn build_manifest(
    root: &Path,
    raw: &CargoToml,
    member_packages: Vec<Package>,
    lockfile: Option<Lockfile>,
) -> Manifest {
    let workspace = convert_workspace(raw, root);
    Manifest::new(root.to_path_buf(), workspace, member_packages, lockfile)
}
