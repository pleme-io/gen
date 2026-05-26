//! `gen-adapter-forge` — typed declarative spec for gen adapters.
//!
//! Every existing adapter (gen-cargo / gen-npm / gen-bundler) has the
//! same shape:
//!
//!   - one or more **manifest markers** (`Cargo.toml`, `package.json`, …)
//!   - zero or more **lockfile markers** (`Cargo.lock`, `pnpm-lock.yaml`, …)
//!   - a **registry** (CratesIo / Npm / RubyGems / …)
//!   - a **constraint syntax family** (`semver-caret-default` /
//!     `semver-exact-default` / `bundler-pessimistic` / …)
//!   - **dependency table names** (`dependencies` / `devDependencies` / …)
//!   - **target-predicate shape** (cargo cfg / npm engines+os / bundler
//!     platforms / pep-508 markers / none)
//!   - **workspace shape** (`members` / `workspaces` array / single-package)
//!
//! A typed [`AdapterSpec`] captures all of this. The forge component
//! generates the boilerplate scaffold (lib.rs + error.rs + raw.rs +
//! tests) from one spec; the adapter author only writes the format-
//! specific parsing logic — typically <100 LOC.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use gen_types::Registry;

/// Typed declarative adapter spec. One value of this struct generates
/// a complete adapter scaffold. Authoring-side equivalent of the
/// `(defadapter …)` Lisp form (M5+).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterSpec {
    /// Adapter name — `cargo`, `npm`, `bundler`, `pip`, `gomod`, ….
    pub name: String,
    /// Crate name on crates.io — convention `gen-<name>`.
    pub crate_name: String,
    /// Marker files probed in declaration order; first hit wins.
    pub manifest_markers: Vec<String>,
    /// Lockfile markers; first present is used, otherwise none.
    pub lockfile_markers: Vec<String>,
    /// Upstream registry the adapter primarily talks to.
    pub registry: Registry,
    /// Constraint-syntax family. See [`ConstraintFamily`].
    pub constraint_family: ConstraintFamily,
    /// Per-kind dependency table name → DependencyKind mapping. e.g.
    /// `dependencies → Direct`, `devDependencies → Dev`, …
    pub dependency_tables: IndexMap<String, String>,
    /// Target-predicate shape supported by this format.
    pub target_predicate_shape: TargetPredicateShape,
    /// Workspace shape — how multi-package projects are declared.
    pub workspace_shape: WorkspaceShape,
    /// Manifest format — drives the parser the forge generates.
    pub manifest_format: ManifestFormat,
    /// Lockfile format — drives the parser the forge generates.
    pub lockfile_format: LockfileFormat,
    /// Human-readable one-line description used in the generated
    /// Cargo.toml `description` field.
    pub description: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConstraintFamily {
    /// Cargo: bare version means caret. `^1.2`, `~1.2`, `>=1.2,<2`.
    SemverCaretDefault,
    /// npm: bare version means exact-match. `^`, `~`, `>=` parsed.
    SemverExactDefault,
    /// Bundler: `~> 1.2` (pessimistic), bare means exact.
    BundlerPessimistic,
    /// pip: `==1.2`, `>=1.2,<2`, `>=1.2.*`. PEP 440.
    Pep440,
    /// Go modules: `v1.2.3` exact + minimum-version-selection.
    GoMvs,
    /// Composer: `^`, `~`, OR-disjunctions (`|`).
    Composer,
    /// Hex: `~> 1.2`, `>= 1.2`. Similar to Bundler.
    HexPessimistic,
    /// No semantic versioning — adapter handles arbitrary strings.
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TargetPredicateShape {
    /// `target.'cfg(unix)'.dependencies` — cargo.
    CargoCfg,
    /// `engines` + `os` + `cpu` — npm.
    NpmEnginesOsCpu,
    /// `platforms` block — Bundler.
    BundlerPlatforms,
    /// PEP-508 environment markers — pip.
    Pep508,
    /// `// +build linux,amd64` build tags — go.
    GoBuildTags,
    /// None — format has no concept of conditional deps.
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkspaceShape {
    /// `[workspace] members = [...]` in a top-level TOML.
    CargoWorkspaceMembers,
    /// `workspaces` array in package.json (or `packages` field).
    NpmWorkspacesField,
    /// Single-package only; no native workspace shape.
    SinglePackageOnly,
    /// Composer monorepo (path repositories).
    ComposerPathRepositories,
    /// pnpm workspace via `pnpm-workspace.yaml`.
    PnpmWorkspaceYaml,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ManifestFormat {
    Toml,
    Json,
    Yaml,
    /// Line-oriented (Gemfile, requirements.txt) — adapter writes a
    /// custom parser.
    LineOriented,
    /// Mixed — adapter sniffs the file extension.
    Sniffed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LockfileFormat {
    Toml,
    Json,
    Yaml,
    /// Line-oriented (Gemfile.lock, requirements.txt freeze).
    LineOriented,
    /// `go.sum` simple two-column format.
    GoSum,
    /// No lockfile.
    None,
}

impl LockfileFormat {
    /// Sniff mode — adapter probes multiple formats. npm prefers
    /// pnpm-lock.yaml over package-lock.json; expressed today as
    /// individual specs choosing the lockfile_format their write-path
    /// settles on.
    pub const fn sniffed_default() -> Self {
        Self::Json
    }
}

// ── Built-in specs (every shipping adapter as typed data) ────────────

/// The cargo adapter as a typed spec — what `gen-cargo` would have
/// been authored as if [`AdapterSpec`] existed before it.
#[must_use]
pub fn cargo_spec() -> AdapterSpec {
    let mut deps = IndexMap::new();
    deps.insert("dependencies".into(), "direct".into());
    deps.insert("dev-dependencies".into(), "dev".into());
    deps.insert("build-dependencies".into(), "build".into());
    AdapterSpec {
        name: "cargo".into(),
        crate_name: "gen-cargo".into(),
        manifest_markers: vec!["Cargo.toml".into()],
        lockfile_markers: vec!["Cargo.lock".into()],
        registry: Registry::CratesIo,
        constraint_family: ConstraintFamily::SemverCaretDefault,
        dependency_tables: deps,
        target_predicate_shape: TargetPredicateShape::CargoCfg,
        workspace_shape: WorkspaceShape::CargoWorkspaceMembers,
        manifest_format: ManifestFormat::Toml,
        lockfile_format: LockfileFormat::Toml,
        description: "gen — Cargo adapter".into(),
    }
}

#[must_use]
pub fn npm_spec() -> AdapterSpec {
    let mut deps = IndexMap::new();
    deps.insert("dependencies".into(), "direct".into());
    deps.insert("devDependencies".into(), "dev".into());
    deps.insert("peerDependencies".into(), "peer".into());
    deps.insert("optionalDependencies".into(), "optional".into());
    AdapterSpec {
        name: "npm".into(),
        crate_name: "gen-npm".into(),
        manifest_markers: vec!["package.json".into()],
        lockfile_markers: vec!["pnpm-lock.yaml".into(), "package-lock.json".into()],
        registry: Registry::Npm,
        constraint_family: ConstraintFamily::SemverExactDefault,
        dependency_tables: deps,
        target_predicate_shape: TargetPredicateShape::NpmEnginesOsCpu,
        workspace_shape: WorkspaceShape::NpmWorkspacesField,
        manifest_format: ManifestFormat::Json,
        lockfile_format: LockfileFormat::sniffed_default(),
        description: "gen — npm/pnpm/yarn adapter".into(),
    }
}

#[must_use]
pub fn bundler_spec() -> AdapterSpec {
    let mut deps = IndexMap::new();
    deps.insert("gem".into(), "direct".into());
    AdapterSpec {
        name: "bundler".into(),
        crate_name: "gen-bundler".into(),
        manifest_markers: vec!["Gemfile".into()],
        lockfile_markers: vec!["Gemfile.lock".into()],
        registry: Registry::RubyGems,
        constraint_family: ConstraintFamily::BundlerPessimistic,
        dependency_tables: deps,
        target_predicate_shape: TargetPredicateShape::BundlerPlatforms,
        workspace_shape: WorkspaceShape::SinglePackageOnly,
        manifest_format: ManifestFormat::LineOriented,
        lockfile_format: LockfileFormat::LineOriented,
        description: "gen — Ruby/Bundler adapter".into(),
    }
}

/// Forge output — the rendered scaffold ready to drop in a new
/// `crates/gen-<name>/` directory.
#[derive(Debug, Clone)]
pub struct ScaffoldOutput {
    pub files: IndexMap<String, String>,
}

/// Forge a fresh adapter scaffold from a typed spec. Produces a
/// directory's worth of starter files:
///   - `Cargo.toml` (with the right deps for the format)
///   - `src/lib.rs` (parse(root) skeleton + dispatch + Manifest assembly)
///   - `src/error.rs` (typed error enum matching the format's failure modes)
///   - `src/raw.rs` (serde shapes for the marker files when JSON/YAML/TOML)
///   - `tests/smoke.rs` (one integration test that asserts the adapter
///     can ingest an empty manifest without panic)
///
/// The output is deliberately a *starter* — the adapter author then
/// fleshes out the format-specific parsing. ~80% of the boilerplate
/// is gone.
#[must_use]
pub fn forge(spec: &AdapterSpec) -> ScaffoldOutput {
    let mut files = IndexMap::new();
    files.insert("Cargo.toml".to_string(), render_cargo_toml(spec));
    files.insert("src/lib.rs".to_string(), render_lib_rs(spec));
    files.insert("src/error.rs".to_string(), render_error_rs(spec));
    if matches!(
        spec.manifest_format,
        ManifestFormat::Json | ManifestFormat::Yaml | ManifestFormat::Toml
    ) {
        files.insert("src/raw.rs".to_string(), render_raw_rs(spec));
    }
    files.insert("tests/smoke.rs".to_string(), render_smoke_test(spec));
    ScaffoldOutput { files }
}

fn render_cargo_toml(s: &AdapterSpec) -> String {
    let format_dep = match s.manifest_format {
        ManifestFormat::Toml => "toml = { workspace = true }\n",
        ManifestFormat::Json => "",
        ManifestFormat::Yaml => "serde_yaml = { workspace = true }\n",
        ManifestFormat::LineOriented | ManifestFormat::Sniffed => "",
    };
    let lock_dep = match s.lockfile_format {
        LockfileFormat::Yaml => "serde_yaml = { workspace = true }\n",
        LockfileFormat::Toml | LockfileFormat::Json | LockfileFormat::LineOriented
        | LockfileFormat::GoSum | LockfileFormat::None => "",
    };
    format!(
        r#"[package]
name = "{crate_name}"
description = "{desc}"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
homepage.workspace = true
repository.workspace = true
authors.workspace = true

[lib]
name = "{lib_name}"
path = "src/lib.rs"

[lints]
workspace = true

[dependencies]
gen-types = {{ workspace = true }}
serde = {{ workspace = true }}
serde_json = {{ workspace = true }}
indexmap = {{ workspace = true }}
thiserror = {{ workspace = true }}
{format_dep}{lock_dep}"#,
        crate_name = s.crate_name,
        lib_name = s.crate_name.replace('-', "_"),
        desc = s.description,
    )
}

fn render_lib_rs(s: &AdapterSpec) -> String {
    let registry_variant = match s.registry {
        Registry::CratesIo => "CratesIo",
        Registry::Npm => "Npm",
        Registry::RubyGems => "RubyGems",
        Registry::PyPi => "PyPi",
        Registry::GoProxy => "GoProxy",
        Registry::Hex => "Hex",
        Registry::Hackage => "Hackage",
        Registry::Packagist => "Packagist",
        Registry::Maven => "Maven",
        Registry::Pub => "Pub",
        Registry::Oci { .. } => "Oci { registry_url: String::new() }",
        Registry::Private { .. } => "Private { url: String::new(), protocol: String::new() }",
        Registry::None => "None",
    };
    let marker = s
        .manifest_markers
        .first()
        .cloned()
        .unwrap_or_else(|| "MANIFEST".into());
    format!(
        r#"//! `{crate_name}` — {name} adapter for the `gen` ecosystem.
//!
//! Generated by gen-adapter-forge from a typed AdapterSpec. Implement
//! the parse_manifest + (optionally) parse_lockfile bodies — the rest
//! of the wiring (manifest assembly, root dispatch, error mapping)
//! is already in place.

pub mod error;
pub use error::{{Error, Result}};

use std::path::Path;

use gen_types::{{
    BuildStep, Dependency, Feature, Lockfile, Manifest, Package, PackageSource, Registry,
    Version, Workspace,
}};

pub const MANIFEST_MARKER: &str = "{marker}";

/// Adapter entrypoint. Reads `<root>/{marker}` and emits a typed Manifest.
pub fn parse(root: &Path) -> Result<Manifest> {{
    let _text = std::fs::read_to_string(root.join(MANIFEST_MARKER)).map_err(|source| Error::Io {{
        path: root.join(MANIFEST_MARKER),
        source,
    }})?;
    // TODO(adapter author): parse _text into Package + Dependency + Feature shapes.
    let placeholder = Package {{
        name: root
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "<unnamed>".to_string()),
        version: Version::new(0, 0, 0),
        source: PackageSource::Path {{
            path: root.display().to_string(),
        }},
        registry: Registry::{registry_variant},
        dependencies: Vec::<Dependency>::new(),
        features: Vec::<Feature>::new(),
        build_steps: Vec::<BuildStep>::new(),
        license: None,
        description: None,
        authors: Vec::new(),
        homepage: None,
        repository: None,
    }};
    let workspace = Workspace::single_package(root.to_path_buf(), "{name}");
    let lockfile = None::<Lockfile>;
    Ok(Manifest::new(root.to_path_buf(), workspace, vec![placeholder], lockfile))
}}
"#,
        crate_name = s.crate_name,
        name = s.name,
        marker = marker,
        registry_variant = registry_variant,
    )
}

fn render_error_rs(s: &AdapterSpec) -> String {
    format!(
        r#"//! Typed errors for the {name} adapter.

use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {{
    #[error("failed to read {{path}}: {{source}}")]
    Io {{
        path: PathBuf,
        #[source]
        source: std::io::Error,
    }},
    #[error("version `{{raw}}` for {{context}} could not be parsed")]
    BadVersion {{ raw: String, context: String }},
    #[error("dependency `{{name}}` requirement `{{raw}}` could not be parsed")]
    BadVersionReq {{ name: String, raw: String }},
}}

pub type Result<T> = std::result::Result<T, Error>;
"#,
        name = s.name,
    )
}

fn render_raw_rs(s: &AdapterSpec) -> String {
    format!(
        r#"//! Raw serde shapes mirroring the {name} on-disk format.
//!
//! TODO(adapter author): replace the placeholder struct with the
//! actual fields the manifest format ships.

use serde::Deserialize;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ManifestRaw {{
    pub name: Option<String>,
    pub version: Option<String>,
}}
"#,
        name = s.name,
    )
}

fn render_smoke_test(s: &AdapterSpec) -> String {
    let marker = s
        .manifest_markers
        .first()
        .cloned()
        .unwrap_or_else(|| "MANIFEST".into());
    let lib_name = s.crate_name.replace('-', "_");
    format!(
        r#"use std::fs;
use std::path::PathBuf;

#[test]
fn ingests_empty_manifest() {{
    let dir: PathBuf = std::env::temp_dir().join("{name}-adapter-smoke");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("{marker}"), "").unwrap();
    let m = {lib_name}::parse(&dir).unwrap();
    assert!(m.package_count() >= 1);
}}
"#,
        name = s.name,
        marker = marker,
        lib_name = lib_name,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cargo_spec_is_well_formed() {
        let s = cargo_spec();
        assert_eq!(s.name, "cargo");
        assert_eq!(s.manifest_markers, vec!["Cargo.toml".to_string()]);
        assert!(matches!(s.constraint_family, ConstraintFamily::SemverCaretDefault));
    }

    #[test]
    fn npm_spec_is_well_formed() {
        let s = npm_spec();
        assert_eq!(s.name, "npm");
        assert!(s.lockfile_markers.contains(&"pnpm-lock.yaml".to_string()));
    }

    #[test]
    fn bundler_spec_is_well_formed() {
        let s = bundler_spec();
        assert!(matches!(s.target_predicate_shape, TargetPredicateShape::BundlerPlatforms));
    }

    #[test]
    fn forge_emits_all_required_files() {
        let s = cargo_spec();
        let out = forge(&s);
        assert!(out.files.contains_key("Cargo.toml"));
        assert!(out.files.contains_key("src/lib.rs"));
        assert!(out.files.contains_key("src/error.rs"));
        assert!(out.files.contains_key("src/raw.rs"));
        assert!(out.files.contains_key("tests/smoke.rs"));
    }

    #[test]
    fn forge_skips_raw_for_line_oriented_format() {
        let s = bundler_spec();
        let out = forge(&s);
        assert!(!out.files.contains_key("src/raw.rs"));
    }

    #[test]
    fn rendered_cargo_toml_has_correct_crate_name() {
        let s = cargo_spec();
        let out = forge(&s);
        let toml = out.files.get("Cargo.toml").unwrap();
        assert!(toml.contains("name = \"gen-cargo\""));
        assert!(toml.contains("name = \"gen_cargo\""));
    }

    #[test]
    fn rendered_lib_uses_correct_registry_variant() {
        let s = npm_spec();
        let out = forge(&s);
        let lib = out.files.get("src/lib.rs").unwrap();
        assert!(lib.contains("Registry::Npm"));
    }

    #[test]
    fn rendered_lib_compiles_to_a_valid_parse_signature() {
        let s = bundler_spec();
        let out = forge(&s);
        let lib = out.files.get("src/lib.rs").unwrap();
        assert!(lib.contains("pub fn parse(root: &Path) -> Result<Manifest>"));
        assert!(lib.contains("MANIFEST_MARKER"));
    }
}
