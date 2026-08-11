//! A variant spec resolves against a DIFFERENT feature set than the
//! canonical one — and writes somewhere else.
//!
//! ## Why this matters
//!
//! gen bakes the RESOLVED feature set into `Cargo.build-spec.json`, and
//! Nix's `rootFeatures` argument is not honored on the lockfile-builder
//! path. The spec IS the feature decision, so there is no way to ask a
//! generated spec for FEWER features than it was generated with. Any
//! crate needing two build variants — with and without an embedded
//! interpreter, say — needs two SPECS.
//!
//! The failure this guards is silent: a variant that resolved to the same
//! features as the canonical spec builds fine, ships, and is simply not
//! the variant anyone asked for. Nothing errors.

use std::path::Path;

use gen_cargo::build_spec::{generate_multi_target_and_write_to, FeatureSelection};

fn write(dir: &Path, rel: &str, body: &str) {
    let p = dir.join(rel);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(p, body).unwrap();
}

/// A workspace with one optional dependency behind a default feature —
/// the shape the real case has.
fn fixture() -> tempfile::TempDir {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path();
    write(
        root,
        "Cargo.toml",
        r#"[package]
name = "variant-probe"
version = "0.1.0"
edition = "2021"

[features]
default = ["plain", "heavy"]
plain = []
heavy = ["dep:libc"]

[dependencies]
libc = { version = "0.2", optional = true }

[workspace]
"#,
    );
    write(root, "src/lib.rs", "pub fn probe() {}\n");
    dir
}

fn features_of(spec_path: &Path) -> Vec<String> {
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(spec_path).unwrap()).unwrap();
    let crates = v.get("crates").and_then(|c| c.as_object()).unwrap();
    let (_, entry) = crates
        .iter()
        .find(|(k, _)| k.starts_with("variant-probe"))
        .expect("root crate present in spec");
    entry
        .get("features")
        .and_then(|f| f.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(ToString::to_string))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn a_variant_spec_subtracts_the_default_feature_set() {
    let dir = fixture();
    let root = dir.path();

    let canonical = generate_multi_target_and_write_to(root, &FeatureSelection::default(), None)
        .expect("canonical spec");
    let variant = generate_multi_target_and_write_to(
        root,
        &FeatureSelection {
            no_default_features: true,
            features: vec!["plain".to_string()],
        },
        Some(Path::new("Cargo.novariant.build-spec.json")),
    )
    .expect("variant spec");

    assert_ne!(canonical, variant, "the variant must not overwrite the canonical spec");
    assert!(variant.ends_with("Cargo.novariant.build-spec.json"));

    let c = features_of(&canonical);
    let v = features_of(&variant);

    assert!(c.contains(&"default".to_string()), "canonical lost `default`: {c:?}");
    assert!(c.contains(&"heavy".to_string()), "canonical lost `heavy`: {c:?}");

    assert!(
        !v.contains(&"default".to_string()),
        "variant still carries `default` — --no-default-features did not reach cargo: {v:?}"
    );
    assert!(
        !v.contains(&"heavy".to_string()),
        "variant still carries `heavy`, so its optional dep is still linked: {v:?}"
    );
    assert!(
        v.contains(&"plain".to_string()),
        "variant dropped the feature it explicitly asked for: {v:?}"
    );
}

/// The delta sidecar names THE spec. Writing a second one for a variant
/// would leave the D2 pre-commit tie unable to say which spec it is
/// judging, and an ambiguous tie is worse than no tie.
#[test]
fn a_variant_spec_does_not_write_a_gen_delta() {
    let dir = fixture();
    let root = dir.path();

    generate_multi_target_and_write_to(
        root,
        &FeatureSelection {
            no_default_features: true,
            features: vec![],
        },
        Some(Path::new("Cargo.variant.build-spec.json")),
    )
    .expect("variant spec");

    assert!(
        !root.join("Cargo.gen.lock").exists(),
        "a variant spec wrote Cargo.gen.lock, making the D2 tie ambiguous"
    );

    // …and the canonical path still does.
    generate_multi_target_and_write_to(root, &FeatureSelection::default(), None)
        .expect("canonical spec");
    assert!(
        root.join("Cargo.gen.lock").exists(),
        "the canonical spec must still emit its delta"
    );
}

#[test]
fn the_default_selection_is_the_historical_behaviour() {
    let d = FeatureSelection::default();
    assert!(d.is_default());
    assert!(!d.no_default_features);
    assert!(d.features.is_empty());
}
