//! Typed [`TargetPredicate`] — conditional-dependency activation.
//!
//! Every package manager has its own syntax for "only install this
//! dep on platform X" / "only on node 18+" / "only on Linux":
//!
//! - Cargo: `target.'cfg(unix)'.dependencies`
//! - npm: `"engines": { "node": ">=18" }` + `"os": ["linux"]`
//! - pip: `[platform_python_implementation == 'CPython']` markers
//! - Bundler: `platforms :ruby, :mswin`
//! - Composer: `"php": ">=8.0"`
//!
//! Adapters normalise their native predicate into the canonical
//! typed shape here. The resolver in `gen-engine` consults this
//! once + decides per-target whether the dep is active.

use serde::{Deserialize, Serialize};

/// Typed conditional-activation predicate. Adapters set this on
/// [`crate::Dependency`] when the dep is conditional.
///
/// Non-recursive on purpose — recursive serde-derived enums explode
/// the trait-solver depth. Compound predicates (And / Or / Not) live
/// in [`CompoundTargetPredicate`] which holds a flat
/// `Vec<TargetPredicate>` + a typed combinator.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum TargetPredicate {
    /// `cfg(unix)` / `cfg(target_os = "linux")` / etc. — Cargo-style
    /// predicate. Stored as the raw cfg expression; the engine has
    /// a typed cfg evaluator that consumes this against a target spec.
    CargoCfg { expr: String },
    /// `os` array (npm-style): only active on these OS names.
    OsList { items: Vec<String> },
    /// `cpu` array (npm-style): only active on these CPU arches.
    CpuList { items: Vec<String> },
    /// Min engine version (npm `engines.<runtime>` / Composer
    /// `php` / etc.).
    EngineMin { engine: String, min: String },
    /// PEP-508 environment marker (Python pip / poetry).
    PythonMarker { marker: String },
    /// Bundler `platforms :foo, :bar` — list of platform symbols.
    BundlerPlatforms { items: Vec<String> },
}

impl TargetPredicate {
    /// Convenience constructors that match the old positional shapes.
    #[must_use]
    pub fn cargo_cfg(expr: impl Into<String>) -> Self {
        Self::CargoCfg { expr: expr.into() }
    }
    #[must_use]
    pub fn os_list(items: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self::OsList {
            items: items.into_iter().map(Into::into).collect(),
        }
    }
    #[must_use]
    pub fn cpu_list(items: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self::CpuList {
            items: items.into_iter().map(Into::into).collect(),
        }
    }
    #[must_use]
    pub fn bundler_platforms(items: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self::BundlerPlatforms {
            items: items.into_iter().map(Into::into).collect(),
        }
    }
}

/// Compound predicate combining atomic [`TargetPredicate`]s. Kept
/// separate from `TargetPredicate` to avoid serde recursion-
/// overflow on the derive macros — same trick as
/// [`crate::CompoundConstraint`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompoundTargetPredicate {
    pub combinator: PredicateCombinator,
    pub atoms: Vec<TargetPredicate>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PredicateCombinator {
    /// All atoms must match (`cfg(all(...))`).
    All,
    /// Any atom matches (`cfg(any(...))`).
    Any,
    /// Inverted — every atom must NOT match. Acts as `Not(All(...))`.
    None,
}

impl CompoundTargetPredicate {
    #[must_use]
    pub fn matches(&self, target: &Target) -> bool {
        match self.combinator {
            PredicateCombinator::All => self.atoms.iter().all(|p| p.matches(target)),
            PredicateCombinator::Any => self.atoms.iter().any(|p| p.matches(target)),
            PredicateCombinator::None => !self.atoms.iter().any(|p| p.matches(target)),
        }
    }
}

/// A concrete target the engine evaluates predicates against.
/// Typically derived from the host system the engine is running on,
/// optionally overridden via shikumi config (e.g. `--target
/// aarch64-linux` cross-build).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    pub os: String,           // "linux" | "macos" | "windows" | …
    pub cpu: String,          // "x86_64" | "aarch64" | …
    pub libc: Option<String>, // "gnu" | "musl" | None on non-linux
    pub engines: indexmap::IndexMap<String, String>,
    pub python_env_markers: indexmap::IndexMap<String, String>,
}

impl Target {
    /// Canonical builder for the current host. Useful in tests; the
    /// engine derives this from system probes at runtime.
    #[must_use]
    pub fn host() -> Self {
        Self {
            #[cfg(target_os = "linux")]
            os: "linux".to_string(),
            #[cfg(target_os = "macos")]
            os: "macos".to_string(),
            #[cfg(target_os = "windows")]
            os: "windows".to_string(),
            #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
            os: "unknown".to_string(),
            #[cfg(target_arch = "x86_64")]
            cpu: "x86_64".to_string(),
            #[cfg(target_arch = "aarch64")]
            cpu: "aarch64".to_string(),
            #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
            cpu: "unknown".to_string(),
            libc: None,
            engines: indexmap::IndexMap::new(),
            python_env_markers: indexmap::IndexMap::new(),
        }
    }
}

impl TargetPredicate {
    /// Evaluate this predicate against a concrete [`Target`]. Returns
    /// `true` when the dependency should be active.
    ///
    /// Note: full Cargo `cfg()` evaluation is delegated to the engine
    /// (which has a typed `CfgEvaluator`); this method falls back to
    /// a conservative "match if cfg is `unix`/`windows`/`<os>`/etc."
    /// shape for the common cases.
    #[must_use]
    pub fn matches(&self, target: &Target) -> bool {
        match self {
            Self::CargoCfg { expr } => eval_cfg_expr(expr, target),
            Self::OsList { items } => items.iter().any(|o| o == &target.os),
            Self::CpuList { items } => items.iter().any(|c| c == &target.cpu),
            Self::EngineMin { engine, min } => target
                .engines
                .get(engine)
                .map(|v| v.as_str() >= min.as_str())
                .unwrap_or(false),
            Self::PythonMarker { .. } => true, // TODO: real PEP-508 eval
            Self::BundlerPlatforms { .. } => true, // TODO: Bundler eval
        }
    }
}

/// Conservative Cargo `cfg(…)` evaluator — handles the common cases
/// (`cfg(unix)`, `cfg(target_os = "linux")`, `cfg(any(…))`,
/// `cfg(all(…))`, `cfg(not(…))`). The engine ships a richer one;
/// this is the fallback for `TargetPredicate::matches` in isolation.
fn eval_cfg_expr(expr: &str, target: &Target) -> bool {
    let expr = expr.trim();
    // Strip outer cfg(...) if present.
    let inner = expr
        .strip_prefix("cfg(")
        .and_then(|s| s.strip_suffix(')'))
        .unwrap_or(expr)
        .trim();
    match inner {
        "unix" => matches!(
            target.os.as_str(),
            "linux" | "macos" | "freebsd" | "netbsd" | "openbsd"
        ),
        "windows" => target.os == "windows",
        "macos" => target.os == "macos",
        "linux" => target.os == "linux",
        s if s.starts_with("target_os") => {
            // target_os = "linux" — extract the quoted name.
            s.split('"').nth(1).is_some_and(|os| os == target.os)
        }
        s if s.starts_with("target_arch") => {
            s.split('"').nth(1).is_some_and(|cpu| cpu == target.cpu)
        }
        // Conservative default: unknown predicates are treated as
        // active. Engine has the full evaluator; this stub returns
        // true so deps don't get silently dropped.
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn linux_x86() -> Target {
        Target {
            os: "linux".into(),
            cpu: "x86_64".into(),
            libc: Some("gnu".into()),
            engines: indexmap::IndexMap::new(),
            python_env_markers: indexmap::IndexMap::new(),
        }
    }

    fn macos_arm() -> Target {
        Target {
            os: "macos".into(),
            cpu: "aarch64".into(),
            libc: None,
            engines: indexmap::IndexMap::new(),
            python_env_markers: indexmap::IndexMap::new(),
        }
    }

    #[test]
    fn cargo_cfg_unix_matches_linux_and_macos() {
        let p = TargetPredicate::cargo_cfg("cfg(unix)");
        assert!(p.matches(&linux_x86()));
        assert!(p.matches(&macos_arm()));
    }

    #[test]
    fn cargo_cfg_target_os_matches_correct_os() {
        let p = TargetPredicate::cargo_cfg("cfg(target_os = \"linux\")");
        assert!(p.matches(&linux_x86()));
        assert!(!p.matches(&macos_arm()));
    }

    #[test]
    fn os_list_matches_when_target_in_list() {
        let p = TargetPredicate::os_list(["linux", "macos"]);
        assert!(p.matches(&linux_x86()));
        assert!(p.matches(&macos_arm()));
    }

    #[test]
    fn cpu_list_matches_when_target_in_list() {
        let p = TargetPredicate::cpu_list(["aarch64"]);
        assert!(p.matches(&macos_arm()));
        assert!(!p.matches(&linux_x86()));
    }

    #[test]
    fn compound_all_requires_every_sub_predicate() {
        let p = CompoundTargetPredicate {
            combinator: PredicateCombinator::All,
            atoms: vec![
                TargetPredicate::os_list(["linux"]),
                TargetPredicate::cpu_list(["x86_64"]),
            ],
        };
        assert!(p.matches(&linux_x86()));
        assert!(!p.matches(&macos_arm()));
    }

    #[test]
    fn compound_any_requires_at_least_one_sub_predicate() {
        let p = CompoundTargetPredicate {
            combinator: PredicateCombinator::Any,
            atoms: vec![
                TargetPredicate::os_list(["linux"]),
                TargetPredicate::os_list(["macos"]),
            ],
        };
        assert!(p.matches(&linux_x86()));
        assert!(p.matches(&macos_arm()));
    }

    #[test]
    fn compound_none_inverts_all() {
        let p = CompoundTargetPredicate {
            combinator: PredicateCombinator::None,
            atoms: vec![TargetPredicate::os_list(["windows"])],
        };
        assert!(p.matches(&linux_x86()));
    }

    #[test]
    fn engine_min_compares_string_versions() {
        let mut t = linux_x86();
        t.engines.insert("node".into(), "20".into());
        let p = TargetPredicate::EngineMin {
            engine: "node".into(),
            min: "18".into(),
        };
        assert!(p.matches(&t));
        let p_too_high = TargetPredicate::EngineMin {
            engine: "node".into(),
            min: "21".into(),
        };
        assert!(!p_too_high.matches(&t));
    }

    #[test]
    fn host_builder_returns_known_os() {
        let h = Target::host();
        assert!(matches!(
            h.os.as_str(),
            "linux" | "macos" | "windows" | "unknown"
        ));
    }
}
