//! Verifies `#[derive(TypedDispatcher)]` reflection over the
//! production `CrateQuirk` enum. The reflection is what substrate
//! emitters consume to produce Nix `helpers = { ... }` skeletons,
//! Lisp catalog entries, and coverage tests mechanically — the
//! P1 starter for the substrate-wide quirk-applier normalization
//! per `theory/QUIRK-APPLIER.md` §IV-bis.3.

use gen_cargo::quirks::CrateQuirk;
use gen_types::TypedDispatcher;

#[test]
fn crate_quirk_variant_kinds_match_serde_tags() {
    let kinds = CrateQuirk::variant_kinds();
    assert_eq!(
        kinds,
        vec![
            "force-cfg",
            "fold-normal-into-build",
            "substitute-source",
            "native-build-inputs"
        ]
    );
}

#[test]
fn crate_quirk_variant_fields_match_struct_shape() {
    let fields = CrateQuirk::variant_fields();
    assert_eq!(
        fields,
        vec![
            ("force-cfg", vec!["cfg"]),
            ("fold-normal-into-build", vec!["extern_crate"]),
            ("substitute-source", vec!["file", "from", "to"]),
            ("native-build-inputs", vec!["packages"]),
        ]
    );
}

#[test]
fn variant_count_matches_kinds_len() {
    assert_eq!(CrateQuirk::variant_count(), 4);
    assert_eq!(
        CrateQuirk::variant_count(),
        CrateQuirk::variant_kinds().len()
    );
}

#[test]
fn reflection_kinds_match_serde_serialized_tags() {
    // Round-trip every variant's serialized JSON kind against the
    // reflected kinds — proves the macro's kebab-case conversion
    // matches `#[serde(rename_all = "kebab-case")]` exactly.
    let samples = [
        CrateQuirk::ForceCfg { cfg: "x".into() },
        CrateQuirk::FoldNormalIntoBuild { extern_crate: None },
        CrateQuirk::SubstituteSource {
            file: "a".into(),
            from: "b".into(),
            to: "c".into(),
        },
        CrateQuirk::NativeBuildInputs {
            packages: vec!["cmake".into()],
        },
    ];
    let reflected = CrateQuirk::variant_kinds();
    for (sample, expected_kind) in samples.iter().zip(reflected.iter()) {
        let v: serde_json::Value = serde_json::to_value(sample).unwrap();
        assert_eq!(v.get("kind").and_then(|k| k.as_str()), Some(*expected_kind));
    }
}
