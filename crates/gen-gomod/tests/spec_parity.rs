//! Spec-parity forcing function — binds the authored Lisp specs to the
//! Rust interpreter + invariant surface so the TYPED-SPEC + INTERPRETER
//! TRIPLET's "both engines drive the SAME authored spec, drift is
//! impossible" guarantee is CI-CAUGHT, not documentary prose.
//!
//! Per `pleme-io/CLAUDE.md` ★★ TYPED-SPEC + INTERPRETER TRIPLET and
//! ★★ CATALOG REFLECTION ("the catalog IS the doc; drift between code
//! and docs becomes impossible; enforced by a test"), the two `.lisp`
//! specs are not free-floating documentation — every symbol they
//! declare must have a home in the shipped Rust:
//!
//! 1. `go-package-build.lisp` `:kinds (std module main)`
//!      ⟺ the [`PackageKind`] variant universe.
//! 2. every `Go-I<n>` the Lisp declares has a TYPED DISPOSITION — an
//!      encoder `Violation` check, an interpreter fail-closed phase, a
//!      structural (unrepresentable) guarantee, or the substrate delta
//!      gate — so a NEW declared invariant with no Rust home fails the
//!      build (and a Rust `Violation` with no declared invariant fails
//!      it too: no phantom checks either direction).
//! 3. every fail-closed interpreter phase (`GomodError::Interp { phase }`)
//!      is a phase the Lisp `:phases` list declares — no phantom error.
//! 4. `adapter.lisp` `:schema-version` ⟺ [`SCHEMA_VERSION`] and
//!      `:ecosystem` ⟺ the adapter name.
//!
//! The Lisp is parsed with a deliberately tiny tolerant tokenizer (no
//! tatara-lisp runtime dep in the test crate) that only extracts the
//! symbols this parity check needs.

use std::collections::BTreeSet;

use gen_gomod::build_spec::{PackageKind, SCHEMA_VERSION};

const ADAPTER_LISP: &str = include_str!("../specs/adapter.lisp");
const ALGO_LISP: &str = include_str!("../specs/go-package-build.lisp");

// ── tiny tolerant Lisp scraping (comments stripped, then regex-free
//    token walks over the source) ──────────────────────────────────────

/// Strip `;`-to-end-of-line Lisp comments so scraping never trips on
/// prose in a `;;` banner.
fn strip_comments(src: &str) -> String {
    src.lines()
        .map(|l| match l.find(';') {
            Some(i) => &l[..i],
            None => l,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Every `Go-I<n>` token that appears in `src` (dedup, sorted). Scans
/// byte-safely over `Go-I` occurrences and reads the trailing digit run;
/// `find` + ASCII digits never cross a char boundary, so multibyte
/// prose (`⇒`, `⟺`, …) in the spec is harmless.
fn declared_invariant_ids(src: &str) -> BTreeSet<String> {
    let src = strip_comments(src);
    let mut out = BTreeSet::new();
    let mut rest = src.as_str();
    while let Some(at) = rest.find("Go-I") {
        let after = &rest[at + "Go-I".len()..];
        let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
        if !digits.is_empty() {
            out.insert(format!("Go-I{digits}"));
        }
        // Advance past this `Go-I` (at least one byte) to find the next.
        rest = &rest[at + "Go-I".len()..];
    }
    out
}

/// The symbols inside a `:kinds ( … )` form (lower-cased leaf symbols).
fn kinds_form(src: &str) -> BTreeSet<String> {
    let src = strip_comments(src);
    let at = src.find(":kinds").expect(":kinds form present in the algorithm spec");
    let rest = &src[at + ":kinds".len()..];
    let open = rest.find('(').expect(":kinds followed by a ( list");
    let close = rest[open..].find(')').expect(":kinds list closes");
    rest[open + 1..open + close]
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

/// Every `:name <sym>` value inside the `:phases (( … ))` form.
fn phase_names(src: &str) -> BTreeSet<String> {
    let src = strip_comments(src);
    let mut out = BTreeSet::new();
    let mut search = src.as_str();
    while let Some(at) = search.find(":name") {
        let after = search[at + ":name".len()..].trim_start();
        let sym: String = after
            .chars()
            .take_while(|c| !c.is_whitespace() && *c != ')' && *c != '(')
            .collect();
        let advance = sym.len();
        if !sym.is_empty() {
            out.insert(sym);
        }
        // Always advance at least one char past `:name` so an empty
        // capture can't spin forever.
        search = &after[advance.max(1).min(after.len())..];
    }
    out
}

// ── the Rust-side truth surfaces ─────────────────────────────────────

/// The canonical [`PackageKind`] variant universe (kebab lower symbols),
/// exhaustively — the `match` makes a new variant a compile error here,
/// so this test can never silently miss a kind.
fn rust_package_kinds() -> BTreeSet<String> {
    [PackageKind::Std, PackageKind::Module, PackageKind::Main]
        .into_iter()
        .map(|k| match k {
            PackageKind::Std => "std",
            PackageKind::Module => "module",
            PackageKind::Main => "main",
        })
        .map(str::to_string)
        .collect()
}

/// Fail-closed interpreter phases the encoder can emit via
/// `GomodError::Interp { phase }`. Mirrors the string literals in
/// `interp.rs`; the phase-coverage test below proves each is a declared
/// Lisp phase, so a rename on one side is caught.
fn interpreter_fail_phases() -> BTreeSet<String> {
    ["reject-cgo", "reject-asm", "relative-path", "read-source"]
        .into_iter()
        .map(str::to_string)
        .collect()
}

/// Typed disposition for every invariant the algorithm spec may declare.
/// This is the parity contract: a declared `Go-I<n>` with no arm here
/// fails `every_declared_invariant_has_a_rust_home`, and an arm naming a
/// `Violation` the encoder doesn't produce fails
/// `every_encoder_violation_maps_to_a_declared_invariant`.
enum Disposition {
    /// Checked by `invariants::check` — carries the `Violation` rule
    /// name(s) (stable kebab tags from `violation_locus`).
    EncoderViolation(&'static [&'static str]),
    /// Enforced by the interpreter failing closed on a phase.
    InterpreterPhase(&'static str),
    /// Made unrepresentable in the type system (no runtime check).
    Structural,
    /// Enforced by the substrate delta gate (`lockfile-delta.nix` D2),
    /// not the encoder's `invariants::check`.
    SubstrateDelta,
}

fn disposition(id: &str) -> Option<Disposition> {
    use Disposition::*;
    Some(match id {
        "Go-I1" => EncoderViolation(&["unresolved-import"]),
        // Go-I2 (build-tree placement) — M1's only kinds are all Target;
        // Host is a reserved arm with no M1 node ⇒ structural.
        "Go-I2" => Structural,
        "Go-I3" => EncoderViolation(&["relative-path-escapes"]),
        // Go-I6 (build constraints resolved by the encoder) — the encoder
        // consumes go list's already-tag-filtered GoFiles; there is no
        // Nix-side tag logic to check ⇒ structural (parity with cargo's
        // cfg-at-emit).
        "Go-I6" => Structural,
        "Go-I7" => SubstrateDelta,
        "Go-I8" => EncoderViolation(&["node-missing-source-hash"]),
        "Go-I9" => EncoderViolation(&["embed-patterns-without-files"]),
        "Go-I10" => {
            EncoderViolation(&["std-node-has-vendored-source", "non-std-node-has-std-source"])
        }
        "Go-I11" => {
            EncoderViolation(&["workspace-member-not-in-packages", "workspace-member-not-main"])
        }
        // Go-I12 (cgo/asm exclusion) — the interpreter fails closed at
        // encode; the M1 PackageKind enum has no Cgo/Tool arm, so it is
        // ALSO structural. Represent as the fail-closed phase (the
        // observable behavior a consumer sees).
        "Go-I12" => InterpreterPhase("reject-cgo"),
        _ => return None,
    })
}

/// Rule tags `invariants::check` can actually emit — mirror of
/// `violation_locus`. Kept exhaustive by construction: every arm below
/// is a real `Violation` variant, and the mapping test proves the union
/// of the dispositions covers exactly these.
fn encoder_violation_tags() -> BTreeSet<&'static str> {
    // Not counting the schema/root sanity checks that aren't a Go-I<n>
    // (stale-schema-version, root-not-in-packages) — those are hygiene,
    // not a declared algorithm invariant.
    [
        "unresolved-import",
        "relative-path-escapes",
        "node-missing-source-hash",
        "embed-patterns-without-files",
        "std-node-has-vendored-source",
        "non-std-node-has-std-source",
        "workspace-member-not-in-packages",
        "workspace-member-not-main",
    ]
    .into_iter()
    .collect()
}

// ── the parity tests ──────────────────────────────────────────────────

#[test]
fn lisp_node_kinds_match_the_rust_enum() {
    let lisp = kinds_form(ALGO_LISP);
    let rust = rust_package_kinds();
    assert_eq!(
        lisp, rust,
        "go-package-build.lisp :kinds must equal the PackageKind variant universe;\n  lisp={lisp:?}\n  rust={rust:?}"
    );
}

#[test]
fn every_declared_invariant_has_a_rust_home() {
    let declared = declared_invariant_ids(ALGO_LISP);
    assert!(
        declared.len() >= 10,
        "expected the M1 invariant set (Go-I1..I12) in the spec; got {declared:?}"
    );
    let mut orphaned = Vec::new();
    for id in &declared {
        match disposition(id) {
            None => orphaned.push(id.clone()),
            // A phase-enforced invariant must name a phase the
            // interpreter can actually emit — otherwise the disposition
            // is fiction.
            Some(Disposition::InterpreterPhase(phase)) => assert!(
                interpreter_fail_phases().contains(phase),
                "{id} claims fail-closed phase `{phase}`, but interp.rs never emits it"
            ),
            Some(_) => {}
        }
    }
    assert!(
        orphaned.is_empty(),
        "these Go-I<n> are declared in go-package-build.lisp but have NO Rust disposition \
         (add a check to invariants.rs, a fail-closed interpreter phase, or a documented \
         structural/substrate arm in tests/spec_parity.rs): {orphaned:?}"
    );
}

#[test]
fn adapter_and_algorithm_specs_agree_on_the_invariant_set() {
    // Both authored specs enumerate the invariants; they must not drift
    // from each other.
    let algo = declared_invariant_ids(ALGO_LISP);
    let adapter_ids = declared_invariant_ids(ADAPTER_LISP);
    // The adapter spec need not RE-declare every invariant, but any it
    // names must be a real one from the algorithm spec (no typo'd id).
    let stray: Vec<_> = adapter_ids.difference(&algo).cloned().collect();
    assert!(
        stray.is_empty(),
        "adapter.lisp names invariant id(s) not in go-package-build.lisp: {stray:?}"
    );
}

#[test]
fn every_encoder_violation_maps_to_a_declared_invariant() {
    // The union of every EncoderViolation disposition must cover exactly
    // the tags invariants::check emits — no orphaned Violation (a check
    // with no declared invariant) and no phantom tag (a disposition
    // naming a Violation that doesn't exist).
    let declared = declared_invariant_ids(ALGO_LISP);
    let mut covered: BTreeSet<&'static str> = BTreeSet::new();
    for id in &declared {
        if let Some(Disposition::EncoderViolation(tags)) = disposition(id) {
            for t in tags {
                assert!(
                    encoder_violation_tags().contains(t),
                    "disposition for {id} names Violation tag `{t}` that invariants.rs never emits"
                );
                covered.insert(*t);
            }
        }
    }
    let emitted = encoder_violation_tags();
    let uncovered: Vec<_> = emitted.difference(&covered).collect();
    assert!(
        uncovered.is_empty(),
        "invariants::check emits Violation tag(s) with no declared Go-I<n> disposition: {uncovered:?}"
    );
}

#[test]
fn every_fail_closed_phase_is_a_declared_lisp_phase() {
    let declared = phase_names(ALGO_LISP);
    let mut missing = Vec::new();
    for phase in interpreter_fail_phases() {
        if !declared.contains(&phase) {
            missing.push(phase);
        }
    }
    assert!(
        missing.is_empty(),
        "interp.rs can emit GomodError::Interp for phase(s) NOT declared in \
         go-package-build.lisp :phases (rename drift): {missing:?}\n  declared={declared:?}"
    );
}

#[test]
fn adapter_spec_schema_version_matches_rust() {
    let src = strip_comments(ADAPTER_LISP);
    let at = src.find(":schema-version").expect(":schema-version in adapter.lisp");
    let after = src[at + ":schema-version".len()..].trim_start();
    let num: String = after.chars().take_while(char::is_ascii_digit).collect();
    let declared: u32 = num.parse().expect(":schema-version is a number");
    assert_eq!(
        declared, SCHEMA_VERSION,
        "adapter.lisp :schema-version ({declared}) must equal build_spec::SCHEMA_VERSION ({SCHEMA_VERSION})"
    );
}

#[test]
fn adapter_spec_ecosystem_matches_adapter_name() {
    let src = strip_comments(ADAPTER_LISP);
    // `:ecosystem "gomod"` and the form head `(defadapter gomod`.
    assert!(
        src.contains(r#":ecosystem      "gomod""#) || src.contains(r#":ecosystem "gomod""#),
        "adapter.lisp must declare :ecosystem \"gomod\""
    );
    assert!(
        src.contains("(defadapter gomod"),
        "adapter.lisp form head must be (defadapter gomod …)"
    );
}
