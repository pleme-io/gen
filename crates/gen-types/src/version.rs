//! Typed [`Version`] — SemVer-compatible. Adapters convert their
//! native version type (cargo's [`semver::Version`], npm's loose
//! pre-release variants, RubyGems' four-segment versions, PEP-440,
//! …) into this canonical shape during parse.

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::fmt;

/// SemVer 2.0.0 compatible version with optional pre-release + build
/// metadata. Major / minor / patch are required; everything past is
/// optional + language-specific extension hooks live in `extension`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    /// `1.2.3-rc.1` → `Some("rc.1")`. Empty string is not legal —
    /// use `None` to indicate no pre-release.
    #[serde(default)]
    pub pre: Option<String>,
    /// `1.2.3+build.42` → `Some("build.42")`. Build metadata is
    /// version-comparison-irrelevant per SemVer (used only for
    /// display).
    #[serde(default)]
    pub build: Option<String>,
    /// Language-specific extension (e.g. RubyGems has a 4th segment;
    /// Python's PEP-440 has epoch + post + dev). Adapter-owned —
    /// the engine doesn't interpret this for comparison purposes.
    #[serde(default)]
    pub extension: Option<String>,
}

impl Version {
    /// Canonical constructor for `MAJOR.MINOR.PATCH` (no pre / build).
    #[must_use]
    pub const fn new(major: u64, minor: u64, patch: u64) -> Self {
        Self {
            major,
            minor,
            patch,
            pre: None,
            build: None,
            extension: None,
        }
    }

    /// Parse a SemVer-shaped string. Returns `None` on malformed
    /// input — adapters that need a typed error should validate at
    /// their boundary.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        // Split off build metadata first (everything after first '+').
        let (without_build, build) = match s.split_once('+') {
            Some((a, b)) => (a, Some(b.to_string())),
            None => (s, None),
        };
        // Then pre-release (everything after first '-').
        let (without_pre, pre) = match without_build.split_once('-') {
            Some((a, b)) => (a, Some(b.to_string())),
            None => (without_build, None),
        };
        let mut parts = without_pre.split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next()?.parse().ok()?;
        let patch = parts.next()?.parse().ok()?;
        // Any extra segments fold into extension (e.g. RubyGems 4th).
        let rest: Vec<&str> = parts.collect();
        let extension = if rest.is_empty() {
            None
        } else {
            Some(rest.join("."))
        };
        Some(Self {
            major,
            minor,
            patch,
            pre,
            build,
            extension,
        })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        // SemVer precedence — major, minor, patch, then pre-release
        // (no pre-release sorts AFTER any pre-release of the same
        // major.minor.patch). Build metadata is comparison-irrelevant.
        match (self.major, self.minor, self.patch).cmp(&(other.major, other.minor, other.patch)) {
            Ordering::Equal => match (&self.pre, &other.pre) {
                (None, None) => Ordering::Equal,
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
                (Some(a), Some(b)) => a.cmp(b),
            },
            other => other,
        }
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if let Some(pre) = &self.pre {
            write!(f, "-{pre}")?;
        }
        if let Some(build) = &self.build {
            write!(f, "+{build}")?;
        }
        if let Some(ext) = &self.extension {
            write!(f, ".{ext}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic_semver() {
        let v = Version::parse("1.2.3").unwrap();
        assert_eq!(v, Version::new(1, 2, 3));
    }

    #[test]
    fn parse_pre_release() {
        let v = Version::parse("1.2.3-rc.1").unwrap();
        assert_eq!(v.pre.as_deref(), Some("rc.1"));
    }

    #[test]
    fn parse_build_metadata() {
        let v = Version::parse("1.2.3+build.42").unwrap();
        assert_eq!(v.build.as_deref(), Some("build.42"));
    }

    #[test]
    fn parse_pre_and_build() {
        let v = Version::parse("1.2.3-rc.1+build.42").unwrap();
        assert_eq!(v.pre.as_deref(), Some("rc.1"));
        assert_eq!(v.build.as_deref(), Some("build.42"));
    }

    #[test]
    fn parse_rubygems_four_segment() {
        // RubyGems: 1.2.3.beta1 → extension captures the 4th segment.
        let v = Version::parse("1.2.3.beta1").unwrap();
        assert_eq!(v.extension.as_deref(), Some("beta1"));
    }

    #[test]
    fn parse_invalid_returns_none() {
        assert!(Version::parse("not-a-version").is_none());
        assert!(Version::parse("1").is_none());
        assert!(Version::parse("1.2").is_none());
    }

    #[test]
    fn ordering_basic() {
        assert!(Version::new(1, 0, 0) < Version::new(1, 0, 1));
        assert!(Version::new(1, 1, 0) > Version::new(1, 0, 99));
        assert!(Version::new(2, 0, 0) > Version::new(1, 99, 99));
    }

    #[test]
    fn ordering_pre_release_sorts_before_release() {
        let pre = Version::parse("1.2.3-rc.1").unwrap();
        let rel = Version::new(1, 2, 3);
        assert!(pre < rel);
    }

    #[test]
    fn display_round_trip() {
        let v = Version::parse("1.2.3-rc.1+build.42").unwrap();
        assert_eq!(format!("{v}"), "1.2.3-rc.1+build.42");
    }

    #[test]
    fn serde_json_round_trip() {
        let v = Version::parse("1.2.3-rc.1").unwrap();
        let j = serde_json::to_string(&v).unwrap();
        let parsed: Version = serde_json::from_str(&j).unwrap();
        assert_eq!(v, parsed);
    }
}
