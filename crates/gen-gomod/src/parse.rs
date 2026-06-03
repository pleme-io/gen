//! Typed `go.mod` + `go.sum` parser.
//!
//! Pure-Rust, no subprocess: reads the two text files at the module
//! root and produces a typed [`GoMod`] / [`GoSum`] pair. This is the
//! gomod analogue of gen-cargo's `parse` (Cargo.toml + Cargo.lock →
//! `gen_types::Manifest`): the build-spec generator (`build_spec.rs`)
//! consumes this typed shape to populate `PackageArgs`.
//!
//! go.mod grammar (the subset gen needs — the full grammar is at
//! <https://go.dev/ref/mod#go-mod-file>):
//!
//! - `module <path>`              — the module's import path.
//! - `go <version>`              — the language version directive.
//! - `require <path> <version>`  — a direct/indirect dep (block or line).
//! - `replace <old> => <new>`    — a module replacement.
//! - `exclude <path> <version>`  — a version exclusion.
//!
//! go.sum grammar: one space-separated triple per line —
//! `<module> <version>[/go.mod] <h1:base64hash>`. Lines ending in
//! `/go.mod` hash the module's go.mod; the bare form hashes the whole
//! module zip. Both are indexed by `(module, version)`.

use std::path::Path;

use indexmap::IndexMap;

use crate::error::{GomodError, Result};

/// A parsed `go.mod` file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GoMod {
    /// The module's import path (`module <path>`).
    pub module: String,
    /// The `go <version>` directive, e.g. `"1.21"`. `None` when absent.
    pub go_version: Option<String>,
    /// `require` directives, in declaration order.
    pub requires: Vec<Require>,
    /// `replace` directives, in declaration order.
    pub replaces: Vec<Replace>,
    /// `exclude` directives, in declaration order.
    pub excludes: Vec<ModuleVersion>,
}

impl GoMod {
    /// The leaf segment of the module path — the conventional `pname`.
    /// `github.com/example/widget` → `widget`. Empty module → `""`.
    #[must_use]
    pub fn pname(&self) -> String {
        self.module
            .rsplit('/')
            .next()
            .unwrap_or(&self.module)
            .to_string()
    }
}

/// A `<module> <version>` pair.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModuleVersion {
    pub path: String,
    pub version: String,
}

/// A `require` directive — a dependency edge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Require {
    pub path: String,
    pub version: String,
    /// `// indirect` marker — a transitive dep not directly imported.
    pub indirect: bool,
}

/// A `replace` directive — `old[ oldver] => new[ newver]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Replace {
    pub old_path: String,
    pub old_version: Option<String>,
    pub new_path: String,
    pub new_version: Option<String>,
}

/// A parsed `go.sum` file: `(module, version) → h1 hash`. The version
/// key preserves the raw form including any `/go.mod` suffix so the
/// two hash kinds (module-zip vs go.mod) never collide.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GoSum {
    /// `(module, version-with-optional-/go.mod-suffix) → h1:<base64>`.
    pub hashes: IndexMap<(String, String), String>,
}

impl GoSum {
    /// Look up the module-zip hash (not the `/go.mod` variant) for a
    /// `(module, version)` pair.
    #[must_use]
    pub fn module_hash(&self, module: &str, version: &str) -> Option<&str> {
        self.hashes
            .get(&(module.to_string(), version.to_string()))
            .map(String::as_str)
    }
}

/// Strip a trailing line comment (`// …`) and surrounding whitespace,
/// returning `(content, had_indirect_marker)`. The indirect marker is
/// the only comment go.mod semantics depend on.
fn strip_comment(line: &str) -> (&str, bool) {
    let indirect = line.contains("// indirect");
    let content = match line.find("//") {
        Some(idx) => &line[..idx],
        None => line,
    };
    (content.trim(), indirect)
}

/// Parse one `require` line body (no `require` keyword, no comment):
/// `<path> <version>`.
fn parse_require(body: &str, indirect: bool) -> Option<Require> {
    let mut it = body.split_whitespace();
    let path = it.next()?.to_string();
    let version = it.next()?.to_string();
    Some(Require {
        path,
        version,
        indirect,
    })
}

/// Parse one `exclude` line body: `<path> <version>`.
fn parse_exclude(body: &str) -> Option<ModuleVersion> {
    let mut it = body.split_whitespace();
    let path = it.next()?.to_string();
    let version = it.next()?.to_string();
    Some(ModuleVersion { path, version })
}

/// Parse one `replace` line body: `old[ oldver] => new[ newver]`.
fn parse_replace(body: &str) -> Option<Replace> {
    let (lhs, rhs) = body.split_once("=>")?;
    let mut l = lhs.split_whitespace();
    let old_path = l.next()?.to_string();
    let old_version = l.next().map(str::to_string);
    let mut r = rhs.split_whitespace();
    let new_path = r.next()?.to_string();
    let new_version = r.next().map(str::to_string);
    Some(Replace {
        old_path,
        old_version,
        new_path,
        new_version,
    })
}

/// Which block (if any) we're inside while scanning a go.mod. Go allows
/// both single-line (`require x v1`) and parenthesized-block
/// (`require (\n  x v1\n)`) forms for require/replace/exclude.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Block {
    None,
    Require,
    Replace,
    Exclude,
}

/// Parse a go.mod's text into the typed [`GoMod`].
#[must_use]
pub fn parse_go_mod_str(text: &str) -> GoMod {
    let mut out = GoMod::default();
    let mut block = Block::None;

    for raw in text.lines() {
        let (content, indirect) = strip_comment(raw);
        if content.is_empty() {
            continue;
        }

        // Close an open parenthesized block.
        if content == ")" {
            block = Block::None;
            continue;
        }

        // Inside an open block: every non-`)` line is a block entry.
        match block {
            Block::Require => {
                if let Some(r) = parse_require(content, indirect) {
                    out.requires.push(r);
                }
                continue;
            }
            Block::Replace => {
                if let Some(r) = parse_replace(content) {
                    out.replaces.push(r);
                }
                continue;
            }
            Block::Exclude => {
                if let Some(e) = parse_exclude(content) {
                    out.excludes.push(e);
                }
                continue;
            }
            Block::None => {}
        }

        // Top-level directives.
        if let Some(rest) = content.strip_prefix("module ") {
            out.module = rest.trim().to_string();
        } else if let Some(rest) = content.strip_prefix("go ") {
            out.go_version = Some(rest.trim().to_string());
        } else if content == "require (" || content == "require(" {
            block = Block::Require;
        } else if let Some(rest) = content.strip_prefix("require ") {
            if let Some(r) = parse_require(rest.trim(), indirect) {
                out.requires.push(r);
            }
        } else if content == "replace (" || content == "replace(" {
            block = Block::Replace;
        } else if let Some(rest) = content.strip_prefix("replace ") {
            if let Some(r) = parse_replace(rest.trim()) {
                out.replaces.push(r);
            }
        } else if content == "exclude (" || content == "exclude(" {
            block = Block::Exclude;
        } else if let Some(rest) = content.strip_prefix("exclude ") {
            if let Some(e) = parse_exclude(rest.trim()) {
                out.excludes.push(e);
            }
        }
        // Unknown directives (`retract`, `toolchain`, …) are ignored —
        // gen doesn't need them for the build spec.
    }

    out
}

/// Parse a go.sum's text into the typed [`GoSum`].
#[must_use]
pub fn parse_go_sum_str(text: &str) -> GoSum {
    let mut hashes: IndexMap<(String, String), String> = IndexMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut it = line.split_whitespace();
        let (Some(module), Some(version), Some(hash)) = (it.next(), it.next(), it.next()) else {
            continue;
        };
        hashes.insert((module.to_string(), version.to_string()), hash.to_string());
    }
    GoSum { hashes }
}

/// Parse the `go.mod` at `<root>/go.mod`. Errors if the file is absent
/// or unreadable.
pub fn parse_go_mod(root: &Path) -> Result<GoMod> {
    let path = root.join("go.mod");
    let text = std::fs::read_to_string(&path).map_err(|_| GomodError::ManifestNotFound(path))?;
    Ok(parse_go_mod_str(&text))
}

/// Parse the `go.sum` at `<root>/go.sum`. Returns an empty [`GoSum`]
/// when the file is absent (a module with no external deps has no
/// go.sum) — that is not an error.
#[must_use]
pub fn parse_go_sum(root: &Path) -> GoSum {
    let path = root.join("go.sum");
    match std::fs::read_to_string(&path) {
        Ok(text) => parse_go_sum_str(&text),
        Err(_) => GoSum::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TESTDATA: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/simple-module");

    #[test]
    fn parses_module_and_go_directive() {
        let m = parse_go_mod(Path::new(TESTDATA)).unwrap();
        assert_eq!(m.module, "github.com/example/widget");
        assert_eq!(m.go_version.as_deref(), Some("1.21"));
        assert_eq!(m.pname(), "widget");
    }

    #[test]
    fn parses_require_block_and_indirect() {
        let m = parse_go_mod(Path::new(TESTDATA)).unwrap();
        // Block requires + single-line indirect require.
        let uuid = m
            .requires
            .iter()
            .find(|r| r.path == "github.com/google/uuid")
            .unwrap();
        assert_eq!(uuid.version, "v1.6.0");
        assert!(!uuid.indirect);
        let spew = m
            .requires
            .iter()
            .find(|r| r.path == "github.com/davecgh/go-spew")
            .unwrap();
        assert!(spew.indirect, "// indirect must be detected");
        assert_eq!(m.requires.len(), 3);
    }

    #[test]
    fn parses_replace_and_exclude() {
        let m = parse_go_mod(Path::new(TESTDATA)).unwrap();
        assert_eq!(m.replaces.len(), 1);
        let r = &m.replaces[0];
        assert_eq!(r.old_path, "golang.org/x/text");
        assert_eq!(r.new_path, "golang.org/x/text");
        assert_eq!(r.new_version.as_deref(), Some("v0.13.0"));
        assert_eq!(m.excludes.len(), 1);
        assert_eq!(m.excludes[0].path, "github.com/google/uuid");
        assert_eq!(m.excludes[0].version, "v1.5.0");
    }

    #[test]
    fn parses_go_sum_hashes() {
        let s = parse_go_sum(Path::new(TESTDATA));
        assert_eq!(
            s.module_hash("github.com/google/uuid", "v1.6.0"),
            Some("h1:NIvaJDMOsjHA8n1jAhLSgzrAzy1Hgr+hNrb57e+94F0=")
        );
        // The /go.mod variant is keyed separately, never collides.
        assert!(s
            .hashes
            .contains_key(&("github.com/google/uuid".into(), "v1.6.0/go.mod".into())));
    }

    #[test]
    fn missing_go_mod_is_error() {
        let tmp = std::env::temp_dir().join("gen-gomod-nonexistent-xyz");
        assert!(parse_go_mod(&tmp).is_err());
    }

    #[test]
    fn single_line_require_form() {
        let m = parse_go_mod_str("module x\ngo 1.20\nrequire foo/bar v1.2.3\n");
        assert_eq!(m.requires.len(), 1);
        assert_eq!(m.requires[0].path, "foo/bar");
        assert_eq!(m.requires[0].version, "v1.2.3");
    }
}
