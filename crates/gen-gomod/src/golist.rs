//! `go list -deps -json` typed model + stream parser.
//!
//! `go list` is the offline resolver — the cargo-metadata analogue.
//! Given a vendored tree (`-mod=vendor`, `GOPROXY=off`) it does the
//! three hard things the encoder never re-implements: build-constraint
//! resolution (`GoFiles` are already tag-filtered for the tuple —
//! Go-I6), `replace`/vendor rewriting (`Dir` points at the replacement,
//! `ImportMap` carries the rewrite), and the transitive dep closure
//! (`-deps` emits one JSON object per package in the closure, std
//! included).
//!
//! Output is a CONCATENATION of JSON objects (not a JSON array), so it
//! is parsed with `serde_json::Deserializer::into_iter` — the same
//! stream shape `go list` has always emitted.

use serde::Deserialize;

use crate::build_spec::PackageKind;
use crate::error::GomodError;

/// One `go list -json` package object. Field names are Go's PascalCase;
/// every list/map/bool field defaults so an omitted-when-empty field
/// (go list's convention) deserializes cleanly.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GoListPackage {
    /// Absolute on-disk directory of the package.
    pub dir: String,
    /// Canonical import path — the node id.
    pub import_path: String,
    /// Package clause name; `"main"` ⇒ a linkable binary.
    #[serde(default)]
    pub name: String,
    /// Module root directory this package lives under. For own packages
    /// AND vendored deps this is the main module's root (vendor/ is
    /// under it); for std it is `$GOROOT/src`.
    #[serde(default)]
    pub root: String,
    /// True ⇒ standard-library package (provided by the shared std
    /// derivation, never per-node source).
    #[serde(default)]
    pub standard: bool,
    /// True ⇒ this package is only in the graph as a dependency (never
    /// a queried root) — so it is never a `workspace_member`.
    #[serde(default)]
    pub dep_only: bool,
    /// Tag-resolved non-test Go sources for this tuple (Go-I6).
    #[serde(default)]
    pub go_files: Vec<String>,
    /// cgo sources. Non-empty on a module/main node ⇒ M1 rejects the
    /// build (Go-I12); M-cgo lifts this to `kind = Cgo`.
    #[serde(default)]
    pub cgo_files: Vec<String>,
    /// Assembly sources. Non-empty on a non-std node ⇒ M1 rejects
    /// (Go-I12); std asm lives inside the opaque std-tree.
    #[serde(default)]
    pub s_files: Vec<String>,
    /// `//go:embed` patterns (drives `-embedcfg`).
    #[serde(default)]
    pub embed_patterns: Vec<String>,
    /// Resolved embed files (relative to `Dir`).
    #[serde(default)]
    pub embed_files: Vec<String>,
    /// Direct imports (the per-node DAG edges). `Deps` (transitive) is
    /// intentionally NOT modeled — the closure is seeded by the full
    /// stream, and per-node importcfg needs only direct edges.
    #[serde(default)]
    pub imports: Vec<String>,
    /// Vendor/replace rewrite: source import path → actual package path.
    #[serde(default)]
    pub import_map: std::collections::BTreeMap<String, String>,
    /// Owning module (own module vs a vendored dep). Absent for std.
    #[serde(default)]
    pub module: Option<GoListModule>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GoListModule {
    #[serde(default)]
    pub path: String,
    /// Absent for the main module; `Some` for a vendored dep.
    #[serde(default)]
    pub version: Option<String>,
    /// True for the main module.
    #[serde(default)]
    pub main: bool,
    #[serde(default)]
    pub go_version: Option<String>,
}

impl GoListPackage {
    /// Classify into the M1 [`PackageKind`]. cgo/asm nodes are NOT
    /// classified here — the encoder rejects them up front (Go-I12).
    #[must_use]
    pub fn kind(&self) -> PackageKind {
        if self.standard {
            PackageKind::Std
        } else if self.name == "main" {
            PackageKind::Main
        } else {
            PackageKind::Module
        }
    }
}

/// Parse the concatenated-JSON-object stream `go list -json` emits.
/// Empty input ⇒ empty vec (a module with no packages).
pub fn parse_stream(json: &str) -> Result<Vec<GoListPackage>, GomodError> {
    let de = serde_json::Deserializer::from_str(json);
    let mut out = Vec::new();
    for item in de.into_iter::<GoListPackage>() {
        let pkg = item.map_err(|e| GomodError::GoListParse(e.to_string()))?;
        out.push(pkg);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Two concatenated objects, exactly as `go list -json` emits them
    // (verified against go1.25 on 2026-07-06).
    const STREAM: &str = r#"
{
	"Dir": "/w/internal/greet",
	"ImportPath": "example.com/fix/internal/greet",
	"Name": "greet",
	"Root": "/w",
	"DepOnly": true,
	"GoFiles": ["greet.go", "greet_linux.go"],
	"EmbedPatterns": ["banner.txt"],
	"EmbedFiles": ["banner.txt"],
	"Imports": ["embed", "fmt"],
	"Module": {"Path": "example.com/fix", "Main": true, "GoVersion": "1.25"}
}
{
	"Dir": "/w/cmd/hello",
	"ImportPath": "example.com/fix/cmd/hello",
	"Name": "main",
	"Root": "/w",
	"GoFiles": ["main.go"],
	"Imports": ["example.com/fix/internal/greet"],
	"Module": {"Path": "example.com/fix", "Main": true}
}
{
	"Dir": "/goroot/src/fmt",
	"ImportPath": "fmt",
	"Name": "fmt",
	"Root": "/goroot/src",
	"Standard": true,
	"DepOnly": true,
	"GoFiles": ["print.go"]
}
"#;

    #[test]
    fn parses_concatenated_object_stream() {
        let pkgs = parse_stream(STREAM).expect("stream parses");
        assert_eq!(pkgs.len(), 3);
        let greet = &pkgs[0];
        assert_eq!(greet.import_path, "example.com/fix/internal/greet");
        assert_eq!(greet.kind(), PackageKind::Module);
        assert!(greet.dep_only);
        assert_eq!(greet.embed_files, vec!["banner.txt"]);
        assert_eq!(pkgs[1].kind(), PackageKind::Main);
        assert_eq!(pkgs[2].kind(), PackageKind::Std);
        assert!(pkgs[2].standard);
    }

    #[test]
    fn empty_stream_is_empty_vec() {
        assert!(parse_stream("").unwrap().is_empty());
        assert!(parse_stream("   \n").unwrap().is_empty());
    }

    #[test]
    fn malformed_stream_errors_not_panics() {
        let err = parse_stream("{not json}").unwrap_err();
        assert!(matches!(err, GomodError::GoListParse(_)));
    }
}
