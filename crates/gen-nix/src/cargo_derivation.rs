//! Per-crate Nix derivation renderer (crate2nix shape) — consumes a
//! [`gen_types::Manifest`] and emits an attrset mapping each
//! `name-version` to its typed derivation.
//!
//! Each derivation is a `buildRustCrate { ... }` call with typed args
//! (crateName / version / src / dependencies / features). The renderer
//! is deterministic: same manifest → byte-identical output.

use std::collections::BTreeSet;

use gen_types::{
    ConstraintSpec, DependencyKind, Lockfile, Manifest, Package, PackageId, PackageSource,
    ResolvedPackage, Version, VersionConstraint,
};

use crate::ast::{entry, AttrSetEntry, NixValue};

/// Top-level entrypoint — render a per-crate derivation attrset for
/// the supplied manifest. Result is a single `NixValue::Lambda` that
/// takes `{ pkgs, buildRustCrate, ... }`.
pub fn render_workspace(manifest: &Manifest) -> NixValue {
    let mut entries: Vec<AttrSetEntry> = Vec::new();

    if let Some(lock) = &manifest.lockfile {
        for resolved in lock.resolved.values() {
            entries.push(entry(
                crate_key(&resolved.id),
                derivation_for_resolved(resolved, lock),
            ));
        }
    }

    for pkg in &manifest.packages {
        // If a workspace member is already in the lockfile, the resolved
        // entry took the source-of-truth slot. Workspace members get an
        // additional path-source override entry under their plain name.
        entries.push(entry(
            crate_key_for_workspace_member(pkg),
            derivation_for_workspace_member(pkg),
        ));
    }

    let crates_attrset = NixValue::AttrSet {
        recursive: false,
        entries,
    };

    let body = NixValue::AttrSet {
        recursive: false,
        entries: vec![
            entry("crates", crates_attrset),
            entry("workspace_members", workspace_member_names(manifest)),
        ],
    };

    NixValue::Lambda {
        params: crate::ast::LambdaParams::Destructured {
            fields: vec![
                crate::ast::ParamField {
                    name: "pkgs".to_string(),
                    default: None,
                },
                crate::ast::ParamField {
                    name: "buildRustCrate".to_string(),
                    default: None,
                },
            ],
            ellipsis: true,
            binding: None,
        },
        body: Box::new(body),
    }
}

fn crate_key(id: &PackageId) -> String {
    format!("{}-{}", id.name, id.version)
}

fn crate_key_for_workspace_member(p: &Package) -> String {
    format!("{}-workspace-{}", p.name, p.version)
}

fn workspace_member_names(manifest: &Manifest) -> NixValue {
    let mut names: BTreeSet<String> = BTreeSet::new();
    for p in &manifest.packages {
        names.insert(crate_key_for_workspace_member(p));
    }
    NixValue::List(names.into_iter().map(NixValue::str).collect())
}

fn derivation_for_resolved(resolved: &ResolvedPackage, lock: &Lockfile) -> NixValue {
    let mut entries: Vec<AttrSetEntry> = Vec::new();
    entries.push(entry("crateName", NixValue::str(&resolved.id.name)));
    entries.push(entry("version", NixValue::str(resolved.id.version.to_string())));
    entries.push(entry("src", source_to_nix(&resolved.source)));
    if let Some(integrity) = &resolved.integrity {
        entries.push(entry("sha256", NixValue::str(integrity.as_str())));
    }
    entries.push(entry(
        "dependencies",
        deps_to_nix(&resolved.resolved_dependencies, lock),
    ));
    let call = NixValue::Apply {
        func: Box::new(NixValue::Ident("buildRustCrate".to_string())),
        args: vec![NixValue::AttrSet {
            recursive: false,
            entries,
        }],
    };
    call
}

fn derivation_for_workspace_member(p: &Package) -> NixValue {
    let mut entries: Vec<AttrSetEntry> = Vec::new();
    entries.push(entry("crateName", NixValue::str(&p.name)));
    entries.push(entry("version", NixValue::str(p.version.to_string())));
    entries.push(entry("src", source_to_nix(&p.source)));
    entries.push(entry(
        "dependencies",
        workspace_member_deps_to_nix(p),
    ));
    if !p.features.is_empty() {
        let features = NixValue::List(
            p.features
                .iter()
                .map(|f| NixValue::str(&f.name))
                .collect(),
        );
        entries.push(entry("features", features));
    }
    NixValue::Apply {
        func: Box::new(NixValue::Ident("buildRustCrate".to_string())),
        args: vec![NixValue::AttrSet {
            recursive: false,
            entries,
        }],
    }
}

fn source_to_nix(src: &PackageSource) -> NixValue {
    match src {
        PackageSource::Registry {
            registry,
            registry_name,
            integrity_hash,
        } => {
            let url = registry_url(registry, registry_name);
            let mut entries = vec![
                entry("kind", NixValue::str("registry")),
                entry("url", NixValue::str(url)),
            ];
            if let Some(h) = integrity_hash {
                entries.push(entry("sha256", NixValue::str(h.as_str())));
            }
            NixValue::AttrSet {
                recursive: false,
                entries,
            }
        }
        PackageSource::Git { url, rev, subdir } => {
            let mut entries = vec![
                entry("kind", NixValue::str("git")),
                entry("url", NixValue::str(url.as_str())),
                entry("rev", NixValue::str(rev.as_str())),
            ];
            if let Some(s) = subdir {
                entries.push(entry("subdir", NixValue::str(s.as_str())));
            }
            NixValue::AttrSet {
                recursive: false,
                entries,
            }
        }
        PackageSource::Path { path } => NixValue::AttrSet {
            recursive: false,
            entries: vec![
                entry("kind", NixValue::str("path")),
                entry("path", NixValue::Path(format_path(path))),
            ],
        },
        PackageSource::Local { path, overrides } => NixValue::AttrSet {
            recursive: false,
            entries: vec![
                entry("kind", NixValue::str("local")),
                entry("path", NixValue::Path(format_path(path))),
                entry("overrides", NixValue::str(overrides.as_str())),
            ],
        },
    }
}

fn registry_url(registry: &gen_types::Registry, name: &str) -> String {
    use gen_types::Registry;
    match registry {
        Registry::CratesIo => format!(
            "https://static.crates.io/crates/{name}/{name}-VERSION.crate"
        ),
        Registry::Private { url, .. } => format!("{url}/{name}"),
        Registry::Npm => format!("https://registry.npmjs.org/{name}"),
        Registry::RubyGems => format!("https://rubygems.org/gems/{name}.gem"),
        Registry::PyPi => format!("https://pypi.org/simple/{name}/"),
        Registry::GoProxy => format!("https://proxy.golang.org/{name}"),
        Registry::Hex => format!("https://repo.hex.pm/tarballs/{name}"),
        Registry::Hackage => format!("https://hackage.haskell.org/package/{name}"),
        Registry::Packagist => format!("https://packagist.org/packages/{name}.json"),
        Registry::Maven => format!("https://repo1.maven.org/maven2/{name}"),
        Registry::Pub => format!("https://pub.dev/api/packages/{name}"),
        Registry::Oci { registry_url } => format!("{registry_url}/{name}"),
        Registry::None => String::new(),
    }
}

fn format_path(p: &str) -> String {
    if p.is_empty() {
        "./.".to_string()
    } else if p.starts_with('/') || p.starts_with("./") {
        p.to_string()
    } else {
        format!("./{p}")
    }
}

fn deps_to_nix(deps: &[PackageId], _lock: &Lockfile) -> NixValue {
    NixValue::List(
        deps.iter()
            .map(|d| NixValue::str(crate_key(d)))
            .collect(),
    )
}

fn workspace_member_deps_to_nix(p: &Package) -> NixValue {
    let mut items: Vec<NixValue> = Vec::new();
    for dep in &p.dependencies {
        if !matches!(
            dep.kind,
            DependencyKind::Direct | DependencyKind::Build | DependencyKind::Optional
        ) {
            continue;
        }
        items.push(NixValue::AttrSet {
            recursive: false,
            entries: vec![
                entry("name", NixValue::str(&dep.name)),
                entry("kind", NixValue::str(dep.kind.as_str())),
                entry("constraint", constraint_to_nix(&dep.constraint)),
            ],
        });
    }
    NixValue::List(items)
}

fn constraint_to_nix(c: &VersionConstraint) -> NixValue {
    let raw = c
        .native_syntax
        .clone()
        .unwrap_or_else(|| spec_to_string(&c.spec));
    NixValue::str(raw)
}

fn spec_to_string(s: &ConstraintSpec) -> String {
    let v = |v: &Version| v.to_string();
    match s {
        ConstraintSpec::Exact(x) => format!("={}", v(x)),
        ConstraintSpec::Range {
            lower_inclusive,
            upper_exclusive,
        } => format!(">={} <{}", v(lower_inclusive), v(upper_exclusive)),
        ConstraintSpec::Tilde(x) => format!("~{}", v(x)),
        ConstraintSpec::Caret(x) => format!("^{}", v(x)),
        ConstraintSpec::GreaterEqual(x) => format!(">={}", v(x)),
        ConstraintSpec::Greater(x) => format!(">{}", v(x)),
        ConstraintSpec::LessEqual(x) => format!("<={}", v(x)),
        ConstraintSpec::Less(x) => format!("<{}", v(x)),
        ConstraintSpec::Any => "*".to_string(),
    }
}
