//! `go.mod` → [`ModuleSpec`] parser.
//!
//! Pure text parse — no network, no `go` invocation. Handles the
//! directives the encoder needs: `module`, `go`, `toolchain`,
//! `require` (single + block), `replace` (single + block). The heavy
//! semantic work — transitive closure, build-constraint resolution,
//! vendor/replace rewriting — is `go list`'s job (see [`crate::golist`]);
//! this parser only produces the module-level [`ModuleSpec`] scalars.

use crate::build_spec::{DepMode, ModuleSpec};

/// One `replace` directive: `old[@version] => new[@version]`. Carried
/// for audit/provenance; `go list` already resolves the effect (it
/// points a replaced package's `Dir` at the replacement).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplaceDirective {
    pub old_path: String,
    pub old_version: Option<String>,
    pub new_path: String,
    pub new_version: Option<String>,
}

/// The parsed shape of a `go.mod`, before target resolution.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GoMod {
    pub module_path: String,
    pub go_version: String,
    pub toolchain: Option<String>,
    /// `(path, version)` per require edge.
    pub requires: Vec<(String, String)>,
    pub replaces: Vec<ReplaceDirective>,
}

impl GoMod {
    /// Parse `go.mod` text. Tolerant of comments, blank lines, and both
    /// single-line + parenthesized-block forms for `require`/`replace`.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let mut m = GoMod::default();
        // `None` = top-level; `Some(Block::Require|Replace)` = inside a
        // parenthesized block whose entries omit the leading keyword.
        let mut block: Option<Block> = None;

        for raw in text.lines() {
            let line = strip_comment(raw).trim();
            if line.is_empty() {
                continue;
            }

            // Close a block.
            if let Some(_b) = block {
                if line == ")" {
                    block = None;
                    continue;
                }
            }

            match block {
                Some(Block::Require) => {
                    if let Some((p, v)) = parse_require_entry(line) {
                        m.requires.push((p, v));
                    }
                    continue;
                }
                Some(Block::Replace) => {
                    if let Some(r) = parse_replace_entry(line) {
                        m.replaces.push(r);
                    }
                    continue;
                }
                None => {}
            }

            // Top-level directives.
            if let Some(rest) = line.strip_prefix("module ") {
                m.module_path = rest.trim().to_string();
            } else if let Some(rest) = line.strip_prefix("go ") {
                m.go_version = rest.trim().to_string();
            } else if let Some(rest) = line.strip_prefix("toolchain ") {
                m.toolchain = Some(rest.trim().to_string());
            } else if line == "require (" || line == "require(" {
                block = Some(Block::Require);
            } else if let Some(rest) = line.strip_prefix("require ") {
                if let Some((p, v)) = parse_require_entry(rest.trim()) {
                    m.requires.push((p, v));
                }
            } else if line == "replace (" || line == "replace(" {
                block = Some(Block::Replace);
            } else if let Some(rest) = line.strip_prefix("replace ") {
                if let Some(r) = parse_replace_entry(rest.trim()) {
                    m.replaces.push(r);
                }
            }
            // `exclude`/`retract` are irrelevant to the build graph — ignored.
        }

        m
    }

    /// Lower the parsed go.mod into the spec's [`ModuleSpec`]. `dep_mode`
    /// is the caller's decision (M1 ⇒ `Vendored`); `vendor_hash` is the
    /// coarse-path fallback (always `None` on the incremental path).
    #[must_use]
    pub fn to_module_spec(&self, dep_mode: DepMode, vendor_hash: Option<String>) -> ModuleSpec {
        ModuleSpec {
            module_path: self.module_path.clone(),
            go_version: self.go_version.clone(),
            toolchain: self.toolchain.clone(),
            has_external_deps: !self.requires.is_empty(),
            dep_mode,
            vendor_hash,
        }
    }
}

#[derive(Clone, Copy)]
enum Block {
    Require,
    Replace,
}

/// Drop a trailing `// …` comment, respecting nothing fancy — go.mod
/// has no string literals so a bare `//` split is correct.
fn strip_comment(line: &str) -> &str {
    match line.find("//") {
        Some(i) => &line[..i],
        None => line,
    }
}

/// `<path> <version>` (block entries and single-line `require` tail).
fn parse_require_entry(s: &str) -> Option<(String, String)> {
    let mut it = s.split_whitespace();
    let path = it.next()?.to_string();
    let version = it.next()?.to_string();
    Some((path, version))
}

/// `<old>[ <oldver>] => <new>[ <newver>]`.
fn parse_replace_entry(s: &str) -> Option<ReplaceDirective> {
    let (lhs, rhs) = s.split_once("=>")?;
    let (old_path, old_version) = split_path_version(lhs.trim());
    let (new_path, new_version) = split_path_version(rhs.trim());
    if old_path.is_empty() || new_path.is_empty() {
        return None;
    }
    Some(ReplaceDirective { old_path, old_version, new_path, new_version })
}

/// `<path>` or `<path> <version>`. A local filesystem replacement
/// (`./foo`, `../bar`) has no version → `(path, None)`.
fn split_path_version(s: &str) -> (String, Option<String>) {
    let mut it = s.split_whitespace();
    let path = it.next().unwrap_or_default().to_string();
    let version = it.next().map(str::to_string);
    (path, version)
}

#[cfg(test)]
mod tests {
    use super::*;

    const AKEYLESS_SHAPED: &str = r#"
module akeyless.io/akeyless-main-repo

go 1.26

toolchain go1.26.4

require (
	github.com/foo/bar v1.2.3
	github.com/baz/qux v0.4.0 // indirect
)

require github.com/single/dep v2.0.0

replace github.com/akeylesslabs/akeyless-go/v3 => ./go/src/client/sdktest/akeyless-go

replace (
	github.com/old/mod v1.0.0 => github.com/new/mod v1.1.0
)
"#;

    #[test]
    fn parses_akeyless_shaped_go_mod() {
        let m = GoMod::parse(AKEYLESS_SHAPED);
        assert_eq!(m.module_path, "akeyless.io/akeyless-main-repo");
        assert_eq!(m.go_version, "1.26");
        assert_eq!(m.toolchain.as_deref(), Some("go1.26.4"));
        assert_eq!(m.requires.len(), 3);
        assert!(m.requires.contains(&("github.com/single/dep".into(), "v2.0.0".into())));
        // in-tree filesystem replace (no version on the RHS).
        let fs_replace = m
            .replaces
            .iter()
            .find(|r| r.old_path == "github.com/akeylesslabs/akeyless-go/v3")
            .expect("in-tree replace present");
        assert_eq!(fs_replace.new_path, "./go/src/client/sdktest/akeyless-go");
        assert_eq!(fs_replace.new_version, None);
        // block replace with versions on both sides.
        let mod_replace = m.replaces.iter().find(|r| r.old_path == "github.com/old/mod").unwrap();
        assert_eq!(mod_replace.old_version.as_deref(), Some("v1.0.0"));
        assert_eq!(mod_replace.new_version.as_deref(), Some("v1.1.0"));
    }

    #[test]
    fn dep_free_module_has_no_external_deps() {
        let m = GoMod::parse("module example.com/x\n\ngo 1.25\n");
        assert!(m.requires.is_empty());
        let spec = m.to_module_spec(DepMode::Vendored, None);
        assert!(!spec.has_external_deps);
        assert_eq!(spec.module_path, "example.com/x");
        assert_eq!(spec.go_version, "1.25");
        assert!(spec.toolchain.is_none());
    }

    #[test]
    fn to_module_spec_carries_dep_mode_and_require_presence() {
        let m = GoMod::parse(AKEYLESS_SHAPED);
        let spec = m.to_module_spec(DepMode::Vendored, None);
        assert!(spec.has_external_deps);
        assert!(spec.dep_mode.is_vendored());
        assert!(spec.vendor_hash.is_none());
    }
}
