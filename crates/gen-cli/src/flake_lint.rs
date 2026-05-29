//! Typed flake-lint primitive.
//!
//! `gen flake-lint <path>` runs `nix flake metadata` on a flake,
//! parses the structured warnings, and emits typed `FlakeIssue`
//! values. `--fix` mode rewrites the offending `.follows` lines
//! to eliminate the issue class.
//!
//! ## Why this lives in the framework, not as a one-off script
//!
//! 1. Typed surface. `FlakeIssue` is a closed enum with the derive
//!    quintet (`TypedDispatcher` + `Discriminant` + `IsVariant`),
//!    making it the 15th typed-dispatcher catalog member
//!    (`gen.flake-lint.issue`). Substrate emitters + downstream
//!    tooling consume the variant universe mechanically.
//! 2. Trait-bounded I/O. The `MetadataSource` trait abstracts "where
//!    does `nix flake metadata` output come from?" — production uses
//!    `NixCliMetadataSource` (subprocess); tests inject a
//!    `StaticMetadataSource` with hand-authored warning text.
//! 3. Reuses gen-cli's existing `format` flag (json | yaml).
//! 4. The fix-on-flake operation is itself a typed function —
//!    same shape as gen-cargo's path-rewrite operations.
//!
//! ## CLI surface
//!
//! ```text
//! gen flake-lint <path>             # report issues; exit 1 if any
//! gen flake-lint <path> --fix       # auto-rewrite offending lines
//! gen --format json flake-lint <p>  # machine-readable report
//! ```
//!
//! ## Substrate composition
//!
//! Substrate's auto-release CI runs `gen flake-lint .` as a gate on
//! every PR. A non-zero exit fails the build; the operator sees the
//! typed FlakeIssue list and either edits flake.nix or runs
//! `gen flake-lint . --fix` to auto-apply.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

/// Typed flake-evaluation issue. Closed enum — every variant is a
/// distinct, mechanically-recognized failure mode at flake-input
/// resolution. New issue classes land here as variants; substrate's
/// fleet-catalog-coverage-test asserts the count.
#[gen_macros::fsm(label = "gen.flake-lint.issue")]
pub enum FlakeIssue {
    /// `inputs.<consumer>.inputs.<target>.follows = "..."` declared
    /// in the parent flake, but `<target>` is NOT an input declared
    /// by `<consumer>`'s actual flake.nix. The override is a no-op
    /// and nix warns. Common after a consumer migrates to the
    /// canonical 3-line form (drops nixpkgs/fenix/devenv inputs).
    StaleFollowsOverride {
        /// The consumer flake whose declaration has the bad override.
        consumer: String,
        /// The phantom input target — the override claims to redirect
        /// this, but the consumer doesn't declare it.
        target: String,
    },
}

/// Trait-bounded source of `nix flake metadata` output. Production
/// implementations spawn `nix` as a subprocess; tests inject a
/// `StaticMetadataSource` with hand-authored text. Same pattern as
/// gen-cargo's `PathDepResolver`.
pub trait MetadataSource: Send + Sync {
    /// Return the combined stdout+stderr of `nix flake metadata`
    /// for the given flake path. Errors surface as Err.
    fn metadata(&self, flake_path: &Path) -> std::io::Result<String>;
}

/// Production impl — spawns `nix flake metadata` and captures
/// combined stdout+stderr.
#[derive(Debug, Default, Clone, Copy)]
pub struct NixCliMetadataSource;

impl MetadataSource for NixCliMetadataSource {
    fn metadata(&self, flake_path: &Path) -> std::io::Result<String> {
        let out = Command::new("nix")
            .args(["flake", "metadata", "--no-write-lock-file"])
            .current_dir(flake_path)
            .output()?;
        let mut combined = String::from_utf8_lossy(&out.stdout).into_owned();
        combined.push_str(&String::from_utf8_lossy(&out.stderr));
        Ok(combined)
    }
}

/// Parse `nix flake metadata` output into typed `FlakeIssue`s.
/// Idempotent — given the same input, always returns the same
/// ordered list of issues. Ordering matches input order so consumers
/// can diff reports across runs.
#[must_use]
pub fn parse_metadata_output(text: &str) -> Vec<FlakeIssue> {
    let mut issues = Vec::new();
    // The warning shape is:
    //   warning: input 'X' has an override for a non-existent input 'Y'
    // Match exactly that pattern — every match is one StaleFollowsOverride.
    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("warning: input '") else {
            continue;
        };
        let Some((consumer, rest)) = rest.split_once('\'') else {
            continue;
        };
        let Some(rest) = rest.strip_prefix(" has an override for a non-existent input '") else {
            continue;
        };
        let Some((target, _)) = rest.split_once('\'') else {
            continue;
        };
        issues.push(FlakeIssue::StaleFollowsOverride {
            consumer: consumer.to_string(),
            target: target.to_string(),
        });
    }
    issues
}

/// Apply the auto-fix for the given issue set against `flake.nix`.
/// Returns the number of lines removed. The fix is conservative:
/// it only deletes `inputs.<target>.follows = "...";` lines INSIDE
/// the relevant `<consumer> = { ... };` block, never touching
/// anything else.
pub fn apply_fix(issues: &[FlakeIssue], flake_path: &Path) -> std::io::Result<usize> {
    let text = std::fs::read_to_string(flake_path)?;
    let (new_text, removed) = rewrite(&text, issues);
    if removed > 0 {
        std::fs::write(flake_path, new_text)?;
    }
    Ok(removed)
}

/// Pure-function rewrite — exposed for testing without touching the
/// filesystem. Given source text + issues, return the rewritten text
/// and a count of removed lines.
#[must_use]
pub fn rewrite(text: &str, issues: &[FlakeIssue]) -> (String, usize) {
    // Build a set of (consumer, target) we want to scrub.
    let mut to_remove: std::collections::HashSet<(String, String)> =
        std::collections::HashSet::new();
    for i in issues {
        match i {
            FlakeIssue::StaleFollowsOverride { consumer, target } => {
                to_remove.insert((consumer.clone(), target.clone()));
            }
        }
    }

    let mut out = String::with_capacity(text.len());
    let mut current_consumer: Option<String> = None;
    let mut removed = 0;
    for line in text.lines() {
        // Detect entry to a consumer block: a line like `<name> = {`
        // at any indentation level. Capture the name.
        let trimmed = line.trim_start();
        if let Some(name) = parse_block_header(trimmed) {
            current_consumer = Some(name);
            out.push_str(line);
            out.push('\n');
            continue;
        }
        // Detect exit from a consumer block: a line like `};` at any
        // indentation level.
        if trimmed == "};" || trimmed == "}" {
            current_consumer = None;
            out.push_str(line);
            out.push('\n');
            continue;
        }
        // Inside a consumer block: maybe drop a follows line.
        if let Some(consumer) = current_consumer.as_ref() {
            if let Some(target) = parse_follows_target(trimmed) {
                if to_remove.contains(&(consumer.clone(), target.clone())) {
                    removed += 1;
                    continue;
                }
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    (out, removed)
}

/// `<name> = {` — return Some(name).
fn parse_block_header(trimmed: &str) -> Option<String> {
    let rest = trimmed.strip_suffix('{')?;
    let rest = rest.trim_end();
    let rest = rest.strip_suffix('=')?;
    let name = rest.trim();
    if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_') {
        return None;
    }
    Some(name.to_string())
}

/// `inputs.<target>.follows = "...";` — return Some(target).
fn parse_follows_target(trimmed: &str) -> Option<String> {
    let rest = trimmed.strip_prefix("inputs.")?;
    let (target, after) = rest.split_once('.')?;
    if !after.starts_with("follows") {
        return None;
    }
    Some(target.to_string())
}

/// Operator-facing run — bundles all three steps: fetch metadata,
/// parse issues, optionally apply fix. Returns the typed report so
/// callers can render it however they want (JSON, YAML, table).
pub fn run<S: MetadataSource>(
    source: &S,
    flake_path: &Path,
    fix: bool,
) -> std::io::Result<FlakeLintReport> {
    let text = source.metadata(flake_path)?;
    let issues = parse_metadata_output(&text);
    let mut fixed = 0usize;
    if fix && !issues.is_empty() {
        let p = flake_path.join("flake.nix");
        fixed = apply_fix(&issues, &p)?;
    }
    Ok(FlakeLintReport {
        flake_path: flake_path.to_path_buf(),
        issues,
        fixed_lines: fixed,
    })
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FlakeLintReport {
    pub flake_path: PathBuf,
    pub issues: Vec<FlakeIssue>,
    /// Count of `.follows` lines removed when `--fix` was passed.
    pub fixed_lines: usize,
}

// Catalog registration emitted by the `#[gen_macros::fsm(...)]` on
// the enum above. 15th consumer class; same algebra as freshness +
// crate-quirk catalogs.

#[cfg(test)]
mod tests {
    use super::*;

    /// Test injection: hand-authored metadata output. Lets every
    /// flake-lint code path execute hermetically.
    struct StaticMetadataSource(&'static str);
    impl MetadataSource for StaticMetadataSource {
        fn metadata(&self, _: &Path) -> std::io::Result<String> {
            Ok(self.0.to_string())
        }
    }

    #[test]
    fn parse_metadata_output_extracts_each_warning() {
        let txt = "\
warning: input 'seibi' has an override for a non-existent input 'fenix'
warning: input 'seibi' has an override for a non-existent input 'nixpkgs'
warning: input 'kontena' has an override for a non-existent input 'nixpkgs'
warning: Git tree is dirty
";
        let issues = parse_metadata_output(txt);
        assert_eq!(issues.len(), 3);
        assert_eq!(
            issues[0],
            FlakeIssue::StaleFollowsOverride {
                consumer: "seibi".into(),
                target: "fenix".into()
            }
        );
        assert_eq!(
            issues[2],
            FlakeIssue::StaleFollowsOverride {
                consumer: "kontena".into(),
                target: "nixpkgs".into()
            }
        );
    }

    #[test]
    fn parse_metadata_output_ignores_unrelated_warnings() {
        let txt = "warning: Git tree is dirty\nwarning: some other thing\n";
        assert!(parse_metadata_output(txt).is_empty());
    }

    #[test]
    fn rewrite_removes_only_targeted_follows_inside_consumer_block() {
        let text = r#"{
  inputs = {
    seibi = {
      url = "github:pleme-io/seibi";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.fenix.follows = "fenix";
      inputs.substrate.follows = "substrate";
    };
    other = {
      inputs.fenix.follows = "fenix";
    };
  };
}
"#;
        let issues = vec![
            FlakeIssue::StaleFollowsOverride {
                consumer: "seibi".into(),
                target: "fenix".into(),
            },
            FlakeIssue::StaleFollowsOverride {
                consumer: "seibi".into(),
                target: "nixpkgs".into(),
            },
        ];
        let (out, removed) = rewrite(text, &issues);
        assert_eq!(removed, 2);
        // seibi keeps substrate.follows, drops nixpkgs + fenix
        assert!(out.contains(r#"inputs.substrate.follows = "substrate";"#));
        assert!(!out.contains("seibi.*inputs.nixpkgs.follows"));
        assert!(!out.contains("seibi.*inputs.fenix.follows"));
        // other's fenix.follows is preserved (different consumer)
        assert!(out.contains(r#"other = {
      inputs.fenix.follows = "fenix";
    };"#));
    }

    #[test]
    fn rewrite_is_idempotent_when_no_issues() {
        let text = "seibi = {\n  inputs.nixpkgs.follows = \"nixpkgs\";\n};\n";
        let (out, removed) = rewrite(text, &[]);
        assert_eq!(removed, 0);
        assert_eq!(out, text);
    }

    #[test]
    fn run_with_mock_source_reports_and_optionally_fixes() {
        let src = StaticMetadataSource(
            "warning: input 'seibi' has an override for a non-existent input 'fenix'\n",
        );
        let tmp = std::env::temp_dir().join(format!(
            "gen-flake-lint-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(
            tmp.join("flake.nix"),
            "seibi = {\n  inputs.fenix.follows = \"fenix\";\n};\n",
        )
        .unwrap();

        let report = run(&src, &tmp, false).unwrap();
        assert_eq!(report.issues.len(), 1);
        assert_eq!(report.fixed_lines, 0);

        let report2 = run(&src, &tmp, true).unwrap();
        assert_eq!(report2.issues.len(), 1);
        assert_eq!(report2.fixed_lines, 1);

        let fixed = std::fs::read_to_string(tmp.join("flake.nix")).unwrap();
        assert!(!fixed.contains("inputs.fenix.follows"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn flake_issue_typed_dispatcher_has_one_variant() {
        use gen_platform::TypedDispatcherTrait;
        assert_eq!(<FlakeIssue as TypedDispatcherTrait>::variant_count(), 1);
        assert_eq!(
            <FlakeIssue as TypedDispatcherTrait>::variant_kinds(),
            vec!["stale-follows-override"]
        );
    }

    #[test]
    fn flake_issue_is_variant_helpers_work() {
        let i = FlakeIssue::StaleFollowsOverride {
            consumer: "x".into(),
            target: "y".into(),
        };
        assert!(i.is_stale_follows_override());
    }

    #[test]
    fn flake_issue_summary_matches_discriminant() {
        let i = FlakeIssue::StaleFollowsOverride {
            consumer: "x".into(),
            target: "y".into(),
        };
        assert_eq!(i.discriminant(), "stale-follows-override");
    }
}
