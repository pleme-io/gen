//! `gen-cache-attic` — typed cache-probe layer for the gen engine.
//!
//! The engine consults a [`CacheClient`] for every derivation before
//! triggering a build. A hit short-circuits the rebuild; a miss flows
//! into the local builder. The trait is the swap-in seam: today's
//! prime impl is [`NoCacheClient`] (always reports miss); the
//! production impl is [`AtticClient`] (HEAD-probes the substituter
//! URL). Future impls (cachix, R2, S3) drop in without touching the
//! engine.

use std::time::Duration;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use gen_config::CacheConfig;
use gen_types::ContentHash;

/// Typed cache key. Engines compute the input-addressed hash of the
/// derivation (typed `Manifest` + adapter + target + features) into a
/// [`ContentHash`] and consult the cache with that key. The key is
/// what makes the cache content-addressed: same inputs → same key →
/// same answer fleet-wide.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CacheKey {
    pub hash: ContentHash,
    /// Stable identifier (`<adapter>/<package>/<version>`) for
    /// diagnostics; not part of the hash, never load-bearing.
    pub label: String,
}

impl CacheKey {
    #[must_use]
    pub fn new(hash: ContentHash, label: impl Into<String>) -> Self {
        Self {
            hash,
            label: label.into(),
        }
    }

    /// Stable store-path-like serialization: `/<hash-hex>-<label>`.
    #[must_use]
    pub fn store_path(&self) -> String {
        format!("/{}-{}", self.hash.hex(), self.label)
    }
}

/// Outcome of a cache probe. Either a hit (substituter has the
/// derivation) or a miss (engine triggers the local builder).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CacheOutcome {
    Hit {
        substituter_url: String,
        /// Optional content size from a HEAD response — useful for
        /// "should we even download this?" decisions on bandwidth-
        /// constrained agents.
        size_bytes: Option<u64>,
    },
    Miss {
        /// Stable miss reason for telemetry. Distinguishes "every
        /// substituter probed, none had it" from "no substituters
        /// configured" from "probe failed transport-level".
        reason: MissReason,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum MissReason {
    /// At least one substituter was probed; none reported the key.
    AllSubstitutersMissing,
    /// Cache lookup was disabled in config (always-build mode).
    AlwaysBuildOverride,
    /// No substituters configured.
    NoSubstituters,
    /// Probe failed with a transport-level error (DNS, TCP, TLS, …).
    /// Engine treats this as a miss + triggers a local build.
    TransportError {
        substituter_url: String,
        detail: String,
    },
}

/// Aggregate report — one probe per supplied key.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CacheReport {
    pub outcomes: IndexMap<String, CacheOutcome>,
}

impl CacheReport {
    #[must_use]
    pub fn hit_count(&self) -> usize {
        self.outcomes
            .values()
            .filter(|o| matches!(o, CacheOutcome::Hit { .. }))
            .count()
    }

    #[must_use]
    pub fn miss_count(&self) -> usize {
        self.outcomes
            .values()
            .filter(|o| matches!(o, CacheOutcome::Miss { .. }))
            .count()
    }

    #[must_use]
    pub fn hit_rate(&self) -> f64 {
        let total = self.outcomes.len();
        if total == 0 {
            return 0.0;
        }
        self.hit_count() as f64 / total as f64
    }
}

/// Typed cache client. New backends (cachix, R2, S3, IPFS, …)
/// implement this trait + plug into the engine via [`gen_config`].
pub trait CacheClient {
    /// Probe a single cache key. Implementations MUST NOT panic;
    /// transport-level errors land as `Miss { TransportError }`.
    fn probe(&self, key: &CacheKey) -> CacheOutcome;

    /// Probe a batch of keys. Default impl loops `probe` — override
    /// for parallel/HTTP-pipelined implementations.
    fn probe_batch(&self, keys: &[CacheKey]) -> CacheReport {
        let mut report = CacheReport::default();
        for k in keys {
            let outcome = self.probe(k);
            report.outcomes.insert(k.label.clone(), outcome);
        }
        report
    }
}

/// No-cache default. Always reports miss. Used when no substituter is
/// configured OR when `cache.always_build = true`.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoCacheClient;

impl CacheClient for NoCacheClient {
    fn probe(&self, _key: &CacheKey) -> CacheOutcome {
        CacheOutcome::Miss {
            reason: MissReason::NoSubstituters,
        }
    }
}

/// Production attic client. Builds an [`CacheClient::probe`] by issuing
/// a HEAD against `<substituter>/<hash>.narinfo` for each configured
/// substituter; first 200-OK wins.
///
/// Today this is a typed stub — the HTTP runtime lives in
/// [`AtticClient::probe`] as a `MissReason::TransportError` when no
/// transport feature is enabled. The real reqwest-backed impl plugs
/// in behind the `http` feature; the engine never knows.
#[derive(Clone, Debug)]
pub struct AtticClient {
    pub substituters: Vec<String>,
    pub trusted_public_keys: Vec<String>,
    pub timeout: Duration,
}

impl AtticClient {
    #[must_use]
    pub fn from_config(cfg: &CacheConfig) -> Self {
        Self {
            substituters: cfg.substituters.clone(),
            trusted_public_keys: cfg.trusted_public_keys.clone(),
            timeout: Duration::from_secs(5),
        }
    }

    fn probe_url(substituter: &str, key: &CacheKey) -> String {
        // Standard nix substituter shape: <base>/<hash>.narinfo
        format!(
            "{}/{}.narinfo",
            substituter.trim_end_matches('/'),
            key.hash.hex()
        )
    }
}

impl CacheClient for AtticClient {
    fn probe(&self, key: &CacheKey) -> CacheOutcome {
        if self.substituters.is_empty() {
            return CacheOutcome::Miss {
                reason: MissReason::NoSubstituters,
            };
        }
        // No HTTP runtime in this base build — every probe reports
        // transport-error miss against the first configured
        // substituter. Real HTTP lives behind the `http` feature
        // (M0.5+ wiring).
        let url = Self::probe_url(&self.substituters[0], key);
        CacheOutcome::Miss {
            reason: MissReason::TransportError {
                substituter_url: url,
                detail: "no HTTP runtime in base build; enable the `http` feature".to_string(),
            },
        }
    }
}

/// Convenience constructor — selects [`NoCacheClient`] or
/// [`AtticClient`] based on a typed [`CacheConfig`]. Engines call this
/// once at startup + cache the boxed result.
#[must_use]
pub fn client_from_config(cfg: &CacheConfig) -> Box<dyn CacheClient + Send + Sync> {
    if cfg.always_build || cfg.substituters.is_empty() {
        Box::new(NoCacheClient)
    } else {
        Box::new(AtticClient::from_config(cfg))
    }
}

#[derive(Debug, Error)]
pub enum CacheError {
    #[error("invalid substituter URL `{url}`: {detail}")]
    InvalidSubstituter { url: String, detail: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use gen_types::ContentHash;

    fn k(label: &str) -> CacheKey {
        CacheKey::new(ContentHash::of(label.as_bytes()), label)
    }

    #[test]
    fn no_cache_client_always_reports_miss() {
        let c = NoCacheClient;
        let outcome = c.probe(&k("foo"));
        assert!(matches!(
            outcome,
            CacheOutcome::Miss {
                reason: MissReason::NoSubstituters
            }
        ));
    }

    #[test]
    fn attic_client_with_no_substituters_misses() {
        let c = AtticClient {
            substituters: vec![],
            trusted_public_keys: vec![],
            timeout: Duration::from_secs(1),
        };
        assert!(matches!(
            c.probe(&k("foo")),
            CacheOutcome::Miss {
                reason: MissReason::NoSubstituters
            }
        ));
    }

    #[test]
    fn attic_client_without_http_reports_transport_error() {
        let c = AtticClient {
            substituters: vec!["https://cache.example.org".to_string()],
            trusted_public_keys: vec![],
            timeout: Duration::from_secs(1),
        };
        let outcome = c.probe(&k("foo"));
        match outcome {
            CacheOutcome::Miss {
                reason:
                    MissReason::TransportError {
                        substituter_url, ..
                    },
            } => {
                assert!(substituter_url.contains("cache.example.org"));
                assert!(substituter_url.ends_with(".narinfo"));
            }
            other => panic!("expected TransportError miss, got {other:?}"),
        }
    }

    #[test]
    fn probe_url_is_substituter_plus_hash_narinfo() {
        let key = k("demo");
        let url = AtticClient::probe_url("https://cache.example.org/foo", &key);
        assert_eq!(
            url,
            format!("https://cache.example.org/foo/{}.narinfo", key.hash.hex())
        );
    }

    #[test]
    fn probe_url_strips_trailing_slash() {
        let key = k("demo");
        let url = AtticClient::probe_url("https://cache.example.org/", &key);
        let after_scheme = url.strip_prefix("https://").unwrap();
        assert!(
            !after_scheme.contains("//"),
            "host+path should not contain `//`: {after_scheme}"
        );
    }

    #[test]
    fn batch_probe_returns_one_outcome_per_key() {
        let c = NoCacheClient;
        let keys = vec![k("a"), k("b"), k("c")];
        let report = c.probe_batch(&keys);
        assert_eq!(report.outcomes.len(), 3);
        assert_eq!(report.miss_count(), 3);
        assert_eq!(report.hit_count(), 0);
        assert_eq!(report.hit_rate(), 0.0);
    }

    #[test]
    fn cache_key_store_path_is_stable() {
        let key = CacheKey::new(ContentHash::of(b"x"), "demo");
        let p = key.store_path();
        assert!(p.starts_with("/"));
        assert!(p.ends_with("-demo"));
    }

    #[test]
    fn client_from_config_selects_no_cache_when_substituters_empty() {
        let cfg = CacheConfig {
            substituters: vec![],
            trusted_public_keys: vec![],
            always_build: false,
        };
        let c = client_from_config(&cfg);
        assert!(matches!(c.probe(&k("foo")), CacheOutcome::Miss { .. }));
    }

    #[test]
    fn client_from_config_respects_always_build_override() {
        let cfg = CacheConfig {
            substituters: vec!["https://x".to_string()],
            trusted_public_keys: vec![],
            always_build: true,
        };
        let c = client_from_config(&cfg);
        // NoCacheClient → NoSubstituters reason.
        let r = c.probe(&k("foo"));
        match r {
            CacheOutcome::Miss {
                reason: MissReason::NoSubstituters,
            } => (),
            other => panic!("expected NoSubstituters miss, got {other:?}"),
        }
    }

    #[test]
    fn hit_rate_computes_for_mixed_outcomes() {
        let mut report = CacheReport::default();
        report.outcomes.insert(
            "a".to_string(),
            CacheOutcome::Hit {
                substituter_url: "x".into(),
                size_bytes: None,
            },
        );
        report.outcomes.insert(
            "b".to_string(),
            CacheOutcome::Miss {
                reason: MissReason::NoSubstituters,
            },
        );
        assert!((report.hit_rate() - 0.5).abs() < 1e-9);
    }
}
