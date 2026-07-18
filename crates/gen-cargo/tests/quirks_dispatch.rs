//! Round-trip tests for the typed `CrateQuirk` registry.
//!
//! Asserts:
//!  - Every registered crate name maps to a non-empty quirk list
//!    (catches accidental registry deletions that leave the lookup
//!    returning empty Vec).
//!  - Every emitted `CrateQuirk` serializes to JSON with the
//!    `kind`-tagged shape the substrate Nix dispatch arms expect.
//!  - Round-trip serde: serialize → deserialize → equal.
//!  - Each known variant (`force-cfg`, `fold-normal-into-build`,
//!    `substitute-source`) is reachable from the registry.

use gen_cargo::quirks::{registered_crate_names, registry, CrateQuirk};

#[test]
fn registry_lookup_is_never_empty_for_registered_names() {
    for name in registered_crate_names() {
        let quirks = gen_cargo::quirks::for_crate(name);
        assert!(
            !quirks.is_empty(),
            "registered name `{name}` resolves to empty quirks — registry corruption"
        );
    }
}

#[test]
fn unknown_crate_resolves_to_empty_quirks() {
    let quirks = gen_cargo::quirks::for_crate("not-a-real-crate-xyz");
    assert!(quirks.is_empty());
}

#[test]
fn every_registered_variant_round_trips_through_json() {
    for (name, quirks) in registry() {
        for q in quirks {
            let json = serde_json::to_string(&q).expect("serialize");
            let back: CrateQuirk = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(q, back, "round-trip lost data for {name}: {json}");
            // The serde tag-key MUST be `kind` (matches Nix dispatch).
            let v: serde_json::Value = serde_json::from_str(&json).unwrap();
            assert!(
                v.get("kind").is_some(),
                "missing `kind` tag in serialization: {json}"
            );
        }
    }
}

#[test]
fn force_cfg_serializes_as_force_cfg_kind() {
    let q = CrateQuirk::ForceCfg {
        cfg: "supports_64bit_atomics".to_string(),
    };
    let v: serde_json::Value = serde_json::to_value(&q).unwrap();
    assert_eq!(v.get("kind"), Some(&serde_json::json!("force-cfg")));
    assert_eq!(
        v.get("cfg"),
        Some(&serde_json::json!("supports_64bit_atomics"))
    );
}

#[test]
fn fold_normal_into_build_serializes_with_extern_crate_field() {
    let with_extern = CrateQuirk::FoldNormalIntoBuild {
        extern_crate: Some("glob".to_string()),
    };
    let v: serde_json::Value = serde_json::to_value(&with_extern).unwrap();
    assert_eq!(
        v.get("kind"),
        Some(&serde_json::json!("fold-normal-into-build"))
    );
    assert_eq!(v.get("extern_crate"), Some(&serde_json::json!("glob")));

    let without = CrateQuirk::FoldNormalIntoBuild { extern_crate: None };
    let v: serde_json::Value = serde_json::to_value(&without).unwrap();
    assert_eq!(v.get("extern_crate"), Some(&serde_json::json!(null)));
}

#[test]
fn substitute_source_carries_all_three_fields() {
    let q = CrateQuirk::SubstituteSource {
        file: "src/foo.rs".to_string(),
        from: "old".to_string(),
        to: "new".to_string(),
    };
    let v: serde_json::Value = serde_json::to_value(&q).unwrap();
    assert_eq!(v.get("kind"), Some(&serde_json::json!("substitute-source")));
    assert_eq!(v.get("file"), Some(&serde_json::json!("src/foo.rs")));
    assert_eq!(v.get("from"), Some(&serde_json::json!("old")));
    assert_eq!(v.get("to"), Some(&serde_json::json!("new")));
}

#[test]
fn native_build_inputs_serializes_with_packages_field() {
    let q = CrateQuirk::NativeBuildInputs {
        packages: vec!["cmake".to_string(), "perl".to_string()],
    };
    let v: serde_json::Value = serde_json::to_value(&q).unwrap();
    assert_eq!(
        v.get("kind"),
        Some(&serde_json::json!("native-build-inputs"))
    );
    assert_eq!(
        v.get("packages"),
        Some(&serde_json::json!(["cmake", "perl"]))
    );
}

#[test]
fn registry_covers_every_class_helper_at_least_once() {
    // The Nix dispatch table has four arms — `force-cfg`,
    // `fold-normal-into-build`, `substitute-source`,
    // `native-build-inputs`. Every arm should be exercised by at
    // least one real registry entry; if not, the dispatch dies for
    // an unused variant the first time somebody adds one.
    let mut seen_force_cfg = false;
    let mut seen_fold = false;
    let mut seen_substitute = false;
    let mut seen_native_build_inputs = false;
    for (_, quirks) in registry() {
        for q in quirks {
            match q {
                CrateQuirk::ForceCfg { .. } => seen_force_cfg = true,
                CrateQuirk::FoldNormalIntoBuild { .. } => seen_fold = true,
                CrateQuirk::SubstituteSource { .. } => seen_substitute = true,
                CrateQuirk::NativeBuildInputs { .. } => seen_native_build_inputs = true,
            }
        }
    }
    assert!(seen_force_cfg, "no ForceCfg quirk in registry");
    assert!(seen_fold, "no FoldNormalIntoBuild quirk in registry");
    assert!(seen_substitute, "no SubstituteSource quirk in registry");
    assert!(
        seen_native_build_inputs,
        "no NativeBuildInputs quirk in registry"
    );
}

#[test]
fn registered_names_are_unique() {
    let mut names: Vec<&str> = registered_crate_names();
    names.sort();
    let original = names.clone();
    names.dedup();
    assert_eq!(
        names.len(),
        original.len(),
        "duplicate crate name in registry: {:?}",
        original
    );
}
