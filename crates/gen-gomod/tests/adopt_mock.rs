//! The adoption census over a mock seam, plus the committed fixtures.
//!
//! Every refusal arm has a fixture because the real fleet has 0 offenders in
//! two of them — `above-fleet-toolchain` and `unparseable` — so without
//! fixtures those arms would be branches nothing ever exercises. That is the
//! same anti-vacuity argument as the directive vector table.

use gen_gomod::adopt::{assess, census, AdoptEnv, AdoptionRefusal, AdoptionVerdict, Census};
use std::collections::HashMap;

struct MockAdoptEnv(HashMap<String, String>);

impl AdoptEnv for MockAdoptEnv {
    fn read_go_mod(&self, root: &str) -> Option<String> {
        self.0.get(root).cloned()
    }
}

fn env() -> MockAdoptEnv {
    let mut m = HashMap::new();
    m.insert("ok".into(), "module example.com/ok\n\ngo 1.25.0\n".into());
    m.insert("bare".into(), "module example.com/bare\n\ngo 1.25\n".into());
    m.insert("above".into(), "module example.com/above\n\ngo 1.27.0\n".into());
    m.insert("nodir".into(), "module example.com/nodir\n".into());
    m.insert("weird".into(), "module example.com/weird\n\ngo 1.2.3.4\n".into());
    MockAdoptEnv(m)
}

const FLEET: &str = "1.26.5";

#[test]
fn each_arm_is_reachable() {
    let e = env();
    assert_eq!(assess(&e, "ok", FLEET), AdoptionVerdict::Eligible);
    assert_eq!(
        assess(&e, "bare", FLEET),
        AdoptionVerdict::Refused(AdoptionRefusal::BareMinorDirective)
    );
    assert_eq!(
        assess(&e, "above", FLEET),
        AdoptionVerdict::Refused(AdoptionRefusal::AboveFleetToolchain)
    );
    assert_eq!(
        assess(&e, "nodir", FLEET),
        AdoptionVerdict::Refused(AdoptionRefusal::NoDirective)
    );
    assert_eq!(
        assess(&e, "weird", FLEET),
        AdoptionVerdict::Refused(AdoptionRefusal::UnparseableDirective)
    );
    // A root with no go.mod at all is a DIFFERENT refusal from an empty
    // directive — conflating them is how "we scanned it" hides "we could not".
    assert_eq!(
        assess(&e, "missing", FLEET),
        AdoptionVerdict::Refused(AdoptionRefusal::NoRootGoMod)
    );
}

#[test]
fn census_prints_a_denominator_and_named_refusals() {
    let e = env();
    let roots: Vec<String> = ["ok", "bare", "above", "nodir", "weird", "missing"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let c = census(&e, &roots, FLEET);

    // The denominator is asserted, not just the offenders. An offenders-only
    // assertion over an EMPTY input set passes silently.
    assert_eq!(c.scanned, 6, "scanned count must be asserted");
    assert_eq!(c.eligible, 1);
    assert_eq!(c.refused.len(), 5);
    assert_eq!(c.bare_minor_directive(), 1);
}

#[test]
fn empty_input_is_not_a_green_census() {
    // The vacuity guard: a census over nothing reports scanned 0, and a caller
    // reading only `eligible == refused.len() == 0` would call that success.
    let c = census(&env(), &[], FLEET);
    assert_eq!(c, Census { scanned: 0, eligible: 0, refused: vec![] });
    assert_eq!(c.scanned, 0, "an empty census must be visibly empty");
}

#[test]
fn committed_fixtures_match_their_arms() {
    // Reads the real files, so the fixtures cannot rot away from the arms.
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/adopt");
    struct Fs(std::path::PathBuf);
    impl AdoptEnv for Fs {
        fn read_go_mod(&self, root: &str) -> Option<String> {
            std::fs::read_to_string(self.0.join(root).join("go.mod")).ok()
        }
    }
    let e = Fs(base);
    assert_eq!(assess(&e, "eligible", FLEET), AdoptionVerdict::Eligible);
    assert_eq!(
        assess(&e, "bare-minor", FLEET),
        AdoptionVerdict::Refused(AdoptionRefusal::BareMinorDirective)
    );
    assert_eq!(
        assess(&e, "above-fleet", FLEET),
        AdoptionVerdict::Refused(AdoptionRefusal::AboveFleetToolchain)
    );
    assert_eq!(
        assess(&e, "no-directive", FLEET),
        AdoptionVerdict::Refused(AdoptionRefusal::NoDirective)
    );
}
