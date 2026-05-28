//! Exercise `#[derive(FromStrKind)]` — the inverse of Discriminant.
//!
//! Round-trip proof: `Discriminant::discriminant() → FromStrKind::from_str
//! → original variant`. This is the typed parse half of the typed-enum
//! reflection toolkit.

use gen_platform::{Discriminant, FromStrKind};
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Discriminant, FromStrKind)]
#[discriminant(method = "kind", case = "kebab")]
#[from_str_kind(case = "kebab")]
enum Color {
    Red,
    Green,
    Blue,
    HotPink,
}

#[test]
fn from_str_parses_each_variant() {
    assert_eq!(Color::from_str("red"), Ok(Color::Red));
    assert_eq!(Color::from_str("green"), Ok(Color::Green));
    assert_eq!(Color::from_str("blue"), Ok(Color::Blue));
    assert_eq!(Color::from_str("hot-pink"), Ok(Color::HotPink));
}

#[test]
fn from_str_unknown_returns_typed_error() {
    let err = Color::from_str("yellow").unwrap_err();
    assert_eq!(err.input, "yellow");
    let msg = format!("{err}");
    assert!(msg.contains("yellow"));
    assert!(msg.contains("red"));
    assert!(msg.contains("hot-pink"));
}

#[test]
fn discriminant_and_from_str_round_trip() {
    // Every variant: variant -> discriminant string -> back to variant
    // must equal the original.
    for variant in [Color::Red, Color::Green, Color::Blue, Color::HotPink] {
        let s = variant.kind();
        let back = Color::from_str(s).unwrap();
        assert_eq!(back, variant, "round-trip failed for {variant:?}");
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Discriminant, FromStrKind)]
#[discriminant(method = "name", case = "title")]
#[from_str_kind(case = "title")]
enum K8sStatus {
    True,
    False,
    Unknown,
}

#[test]
fn case_title_round_trips_k8s_wire_format() {
    // K8s metav1.ConditionStatus uses "True"/"False"/"Unknown"
    // verbatim — case=title preserves the variant identifier.
    assert_eq!(K8sStatus::True.name(), "True");
    assert_eq!(K8sStatus::from_str("False"), Ok(K8sStatus::False));
    assert_eq!(K8sStatus::from_str("Unknown"), Ok(K8sStatus::Unknown));
    assert!(K8sStatus::from_str("true").is_err()); // case-sensitive
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Discriminant, FromStrKind)]
#[discriminant(method = "kind", case = "kebab")]
#[from_str_kind(case = "kebab")]
enum WebhookKind {
    Generic,
    GenericHmac,
    #[discriminant(name = "dockerhub")]
    #[from_str_kind(name = "dockerhub")]
    DockerHub,
    Quay,
}

#[test]
fn per_variant_override_works_both_directions() {
    assert_eq!(WebhookKind::DockerHub.kind(), "dockerhub");
    assert_eq!(WebhookKind::from_str("dockerhub"), Ok(WebhookKind::DockerHub));
    assert!(WebhookKind::from_str("docker-hub").is_err());
}
