//! Typed BundlerQuirk registry — upstream-gem build-time workarounds
//! for the nixpkgs `bundlerEnv` / `bundlerApp` sandbox.

use serde::{Deserialize, Serialize};

/// Quirks for known third-party Ruby gems whose install/build fails
/// inside the nixpkgs sandbox without a known-good workaround.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum BundlerQuirk {
    /// Pin a specific Ruby interpreter for a gem whose native
    /// extensions break on the default. Targets a gemspec's
    /// native build phase.
    PinRuby { version: String },
    /// Skip a gem's native compilation step (use the pure-Ruby fallback).
    /// Useful when a gem has both a C and pure-Ruby implementation
    /// (e.g. `eventmachine`, `bcrypt`) and the C path triggers sandbox
    /// access denials.
    SkipNativeBuild,
    /// Force a CPU/feature flag during native build. Some gems
    /// (`grpc`, `google-protobuf`) need `-mssse3` etc on x86.
    ExtraCflags { flags: String },
    /// Substitute a one-line source patch — analog of CrateQuirk +
    /// NpmQuirk's SubstituteSource for Ruby source bugs whose fix is
    /// a trivially small string replacement.
    SubstituteSource {
        file: String,
        from: String,
        to: String,
    },
    /// Override the gem's source URL (e.g. switch from a broken
    /// github archive to rubygems.org).
    OverrideSource { url: String },
}

/// Canonical Bundler quirks registry. Empty for now — entries land
/// as we encounter real upstream gems needing each class.
pub fn registry() -> Vec<(&'static str, Vec<BundlerQuirk>)> {
    Vec::new()
}

#[derive(gen_macros::QuirkRegistry)]
#[quirks(enum_name = "BundlerQuirk", registry_fn = "crate::quirks::registry")]
pub struct BundlerQuirks;
