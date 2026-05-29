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
    /// `url = "github:pleme-io/<input>/<rev>"` rev-pinned URL in
    /// flake.nix. Freezes the input at a historical rev; subsequent
    /// `nix flake update <input>` calls can't move it because the
    /// URL itself contains the pin. Substrate-grade fixes shipped to
    /// the input's main are invisible to consumers behind a rev-
    /// pinned URL. Doctrine (substrate/CLAUDE.md): pleme-io fleet
    /// flakes do NOT rev-pin internal pleme-io URLs.
    StaleInternalPin {
        /// The input name being rev-pinned (e.g. "substrate", "blackmatter-kubernetes").
        input: String,
        /// The historical rev hard-coded in the URL.
        rev: String,
        /// The line of flake.nix where the offending URL was found
        /// (1-indexed). Useful for the `--fix` autofix to target.
        line: u32,
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
/// Returns the number of fixed lines (deleted follows + de-pinned
/// URLs). Conservative: only edits the exact lines the issues
/// target; never touches anything else.
pub fn apply_fix(issues: &[FlakeIssue], flake_path: &Path) -> std::io::Result<usize> {
    let text = std::fs::read_to_string(flake_path)?;
    let (new_text, removed) = rewrite(&text, issues);
    if new_text != text {
        std::fs::write(flake_path, new_text)?;
    }
    Ok(removed)
}

/// Parse a single flake.nix file for `StaleInternalPin` issues —
/// `github:pleme-io/<input>/<rev>` URLs that hard-pin the rev.
/// Pure-function scan: no I/O, no nix-eval, no subprocess.
#[must_use]
pub fn parse_flake_nix_pins(text: &str) -> Vec<FlakeIssue> {
    let mut out = Vec::new();
    let prefix = "github:pleme-io/";
    for (i, line) in text.lines().enumerate() {
        // Look for `url = "github:pleme-io/<input>/<rev>"`. Conservative:
        // require url = "... in the same line + the prefix + a
        // /<hex> suffix.
        let line_trim = line.trim();
        if !line_trim.contains("url") || !line_trim.contains(prefix) {
            continue;
        }
        let Some(quote_open) = line.find(prefix) else { continue; };
        let after = &line[quote_open + prefix.len()..];
        let Some(quote_close) = after.find('"') else { continue; };
        let url_body = &after[..quote_close];
        let Some(slash) = url_body.find('/') else { continue; };
        let input = &url_body[..slash];
        let rev_raw = &url_body[slash + 1..];
        // Strip any ?args= or branch params.
        let rev = rev_raw.split('?').next().unwrap_or(rev_raw);
        // Heuristic: a rev is 6-40 hex chars. Branch names like
        // "main", "develop" are excluded.
        if rev.len() < 6 || rev.len() > 40 || !rev.chars().all(|c| c.is_ascii_hexdigit()) {
            continue;
        }
        out.push(FlakeIssue::StaleInternalPin {
            input: input.to_string(),
            rev: rev.to_string(),
            line: (i + 1) as u32,
        });
    }
    out
}

/// Pure-function rewrite — exposed for testing without touching the
/// filesystem. Given source text + issues, return the rewritten text
/// and a count of removed lines.
#[must_use]
pub fn rewrite(text: &str, issues: &[FlakeIssue]) -> (String, usize) {
    // Two-step rewrite. First pass: drop StaleFollowsOverride lines.
    // Second pass: depin StaleInternalPin URLs (strip `/<rev>` from
    // `github:pleme-io/<input>/<rev>`).
    let mut to_remove: std::collections::HashSet<(String, String)> =
        std::collections::HashSet::new();
    let mut to_depin: std::collections::HashSet<(String, String)> =
        std::collections::HashSet::new();
    for i in issues {
        match i {
            FlakeIssue::StaleFollowsOverride { consumer, target } => {
                to_remove.insert((consumer.clone(), target.clone()));
            }
            FlakeIssue::StaleInternalPin { input, rev, .. } => {
                to_depin.insert((input.clone(), rev.clone()));
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
        // URL-depin pass: strip `/<rev>` from
        // `github:pleme-io/<input>/<rev>` when (input, rev) is in
        // to_depin. Conservative: only the exact (input, rev) pair
        // gets edited; other rev-pinned URLs (external orgs, etc)
        // are untouched.
        let mut depinned_line = line.to_string();
        let mut line_changed = false;
        for (input, rev) in &to_depin {
            let needle = format!("github:pleme-io/{input}/{rev}");
            let replacement = format!("github:pleme-io/{input}");
            if depinned_line.contains(&needle) {
                depinned_line = depinned_line.replace(&needle, &replacement);
                line_changed = true;
            }
        }
        if line_changed {
            removed += 1;
        }
        out.push_str(&depinned_line);
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
    let mut issues = parse_metadata_output(&text);
    // Also scan flake.nix directly for rev-pinned internal URLs.
    let flake_nix = flake_path.join("flake.nix");
    if flake_nix.exists() {
        let flake_text = std::fs::read_to_string(&flake_nix)?;
        issues.extend(parse_flake_nix_pins(&flake_text));
    }
    let mut fixed = 0usize;
    if fix && !issues.is_empty() {
        fixed = apply_fix(&issues, &flake_nix)?;
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
    fn parse_flake_nix_pins_detects_rev_pinned_urls() {
        let text = r#"
{
  inputs = {
    substrate = {
      url = "github:pleme-io/substrate/3594ae2dce08";
    };
    nixpkgs.url = "github:nixos/nixpkgs/main";
    blackmatter-kubernetes = {
      url = "github:pleme-io/blackmatter-kubernetes/99ba10a";
    };
    upstream.url = "github:some-other-org/repo/deadbeef";
  };
}
"#;
        let issues = parse_flake_nix_pins(text);
        assert_eq!(issues.len(), 2, "expected 2 pleme-io pins, got: {:?}", issues);
        let names: Vec<_> = issues
            .iter()
            .filter_map(|i| match i {
                FlakeIssue::StaleInternalPin { input, .. } => Some(input.as_str()),
                _ => None,
            })
            .collect();
        assert!(names.contains(&"substrate"));
        assert!(names.contains(&"blackmatter-kubernetes"));
    }

    #[test]
    fn parse_flake_nix_pins_ignores_non_pleme_io_urls() {
        let text = r#"
{
  inputs.upstream.url = "github:some-other-org/repo/deadbeefcafebabe";
  inputs.nixpkgs.url = "github:nixos/nixpkgs/main";
}
"#;
        assert!(parse_flake_nix_pins(text).is_empty());
    }

    #[test]
    fn rewrite_depins_internal_url_only_when_issue_targets_it() {
        let text = r#"
{
  inputs = {
    substrate.url = "github:pleme-io/substrate/3594ae2";
    untouched.url = "github:pleme-io/untouched/abcdef0";
  };
}
"#;
        let issues = vec![FlakeIssue::StaleInternalPin {
            input: "substrate".into(),
            rev: "3594ae2".into(),
            line: 4,
        }];
        let (out, removed) = rewrite(text, &issues);
        assert_eq!(removed, 1);
        assert!(out.contains(r#"substrate.url = "github:pleme-io/substrate";"#));
        // untouched stays pinned (no issue targets it)
        assert!(out.contains(r#"untouched.url = "github:pleme-io/untouched/abcdef0";"#));
    }

    #[test]
    fn flake_issue_now_has_two_variants() {
        use gen_platform::TypedDispatcherTrait;
        assert_eq!(<FlakeIssue as TypedDispatcherTrait>::variant_count(), 2);
        let kinds = <FlakeIssue as TypedDispatcherTrait>::variant_kinds();
        assert!(kinds.contains(&"stale-follows-override"));
        assert!(kinds.contains(&"stale-internal-pin"));
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
    fn flake_issue_typed_dispatcher_variant_count_is_two() {
        // Two issue classes today: StaleFollowsOverride +
        // StaleInternalPin. Asserts in lockstep with the
        // substrate fleet-catalog-coverage-test snapshot.
        use gen_platform::TypedDispatcherTrait;
        assert_eq!(<FlakeIssue as TypedDispatcherTrait>::variant_count(), 2);
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
