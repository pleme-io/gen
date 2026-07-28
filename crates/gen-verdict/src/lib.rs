//! `gen-verdict` — the **derived-verdict law** as a type.
//!
//! > A verdict is derived from the subject set it claims about, and carries
//! > the witness of that derivation.
//!
//! Canonical statement: [`theory/UNREPRESENTABILITY.md`](https://github.com/pleme-io/theory/blob/main/UNREPRESENTABILITY.md)
//! §II.4. This crate is that law's first fleet-wide implementation. It mints
//! no new name and no new vocabulary — `Subjects`, `Domain`, `Verdict`,
//! `judge`, `Vacuous`, `Unreached` are the doctrine's own words.
//!
//! ## The two clauses, independent and both required
//!
//! 1. **Derivation.** The verdict is a total function of the examined
//!    subject set. One classifier computes it; no constructor accepts a
//!    verdict. Here: [`Verdict::judge`], with `Held` and `Falsified` marked
//!    `#[non_exhaustive]` so no downstream crate can name them.
//! 2. **Witness.** The verdict carries that subject set. The pass arm is
//!    non-constructible without a **non-empty** subject-set witness, and the
//!    empty case is a **distinct arm**. Here: [`Verdict::Held`] names a
//!    [`Subjects<S>`], which is [`NonEmpty<T>`], which has no empty
//!    inhabitant; and [`Verdict::Vacuous`] is its own arm.
//!
//! Each clause alone fails. Derivation without a witness is correct and
//! unauditable — a reader cannot tell a pass over 300 subjects from a pass
//! over one. A witness without derivation is auditable and assertable — a
//! caller constructs `Held { subjects }` with any set it likes, including
//! one it never examined.
//!
//! ## Clause 2 does not say "empty fails"
//!
//! It says emptiness is **sayable**, so its meaning is decided once,
//! visibly, per domain. magma's `Coverage::Vacuous` supports an in-sync
//! claim — an empty world cannot drift. `skill-lint`'s zero-skill run is an
//! error — a linter that validated nothing validated nothing. Both are
//! right; neither is expressible while emptiness is silently a pass. This
//! crate gives the caller [`Verdict::Vacuous`] to match on and decides
//! nothing on their behalf.
//!
//! ## The witness is a value, not a count
//!
//! A `usize` is assertable. Even a `NonZeroUsize` fixes zero while carrying
//! no provenance — it says *some* number of things was examined, and cannot
//! say which. Only a value obtainable *by having the subjects* makes the
//! count and the claim share one origin, so [`Subjects<S>`] carries the
//! examined values and [`NonEmpty::count`] is a projection of them.
//!
//! ## Usage
//!
//! ```
//! use gen_verdict::{Subjects, Verdict};
//!
//! # struct Rule;
//! # fn violations_of(_: &str) -> Vec<String> { Vec::new() }
//! fn check(files: Vec<String>) -> Verdict<String, String> {
//!     let findings: Vec<String> = files.iter().flat_map(|f| violations_of(f)).collect();
//!     Verdict::judge(Subjects::scope(files), findings)
//! }
//!
//! // Zero files is not a pass — it is a nameable, distinct fact.
//! assert_eq!(check(Vec::new()), Verdict::Vacuous);
//!
//! // A pass carries what it examined.
//! let v = check(vec!["a.rs".into(), "b.rs".into()]);
//! assert!(v.is_held());
//! assert_eq!(v.subject_count(), 2);
//!
//! // And a guarded action can require the proof as an argument.
//! let permit = v.into_permit().expect("held");
//! let published = permit.authorize(|examined| examined.count().get());
//! assert_eq!(published, 2);
//! ```
//!
//! ## Tier — graded, not rounded
//!
//! Per the fleet bright line: a `Result::Err` is **mitigation**; a compile
//! error / absent method / absent field is **unrepresentability**. Never
//! conflate them, never round up.
//!
//! | Claim | Tier | Why |
//! |---|---|---|
//! | a pass without a non-empty witness | **truly-unrepresentable** | [`NonEmpty<T>`] has no empty inhabitant and [`Verdict::Held`] names the field. No expressible program constructs it |
//! | the `Held` / `Falsified` arm named outside this crate | **truly-unrepresentable** | both are `#[non_exhaustive]`: a struct-expression is `E0639` |
//! | a pass alongside findings | **truly-unrepresentable downstream**; **only-mitigated in-crate** | [`Verdict::judge`] is the sole classifier. Inside this crate the arms are nameable; Rust cannot express "only this function may name this variant" beyond module and crate privacy |
//! | a forged verdict arriving over a wire | **parse-time-rejected** | `#[serde(try_from)]` re-derives from the record's own witness and returns [`VerdictError`]. A `Result::Err`, not a compile error |
//! | a guarded action reachable without a held verdict | **truly-unrepresentable** | [`Permit<S>`] has no public constructor and its sole minter is [`Verdict::into_permit`] |
//! | the action is *handed* a set other than the examined one | **truly-unrepresentable** | [`Permit::authorize`] moves the permit's own witness into the action |
//! | the action *operates on* the set it was handed | **only-mitigated** | no type expresses "this argument must be used". See [`permit`]'s residual table |
//! | the subject set is the **right** set | **only-mitigated** | non-emptiness is silent about scope: a `Subjects` of the wrong three items is as well-typed as the right three hundred. §II.3's E discipline is unreplaced |
//! | the witness came from the thing claimed about | **only-mitigated, C2-shaped** | §II.3's F ceiling, unchanged. A transcribed set and a derived set are values of the same type, separated only by provenance, which the program does not carry |
//! | a no-claim arm asserted when a claim was available (round-**down**) | **only-mitigated** | [`Verdict::Vacuous`] and [`Verdict::Unreached`] are freely constructible, because a caller legitimately needs to record "nothing in scope" and "never ran". Nothing stops a caller naming one while holding 300 examined subjects. §VIII.1 notes derive-not-assert is symmetric and that round-down is currently uncovered fleet-wide; sealing the no-claim arms would cost the honest default and is not obviously the right trade |
//! | **the gate has a caller at all** | **NOT COVERED** | see below |
//!
//! ### What this crate does **not** cover
//!
//! §II.4 grades the law across §II.3's six vacuous-guard subclasses and
//! covers five. **Subclass B — Unreached — is explicitly not one of them,
//! and this crate does not cover it either.** Reachability is a property of
//! the call graph, not of a value: [`Verdict::Unreached`] is a fact
//! something that *ran* can record, and a gate nobody calls constructs
//! nothing at all — including nothing that says so. The cure stays a
//! zero-caller / unreachable-code lint on the guard's own body. Naming this
//! is load-bearing: an umbrella that claims everything covers nothing.
//!
//! The five it does address:
//!
//! | §II.3 subclass | Covered by |
//! |---|---|
//! | **A. Vacuous** | [`NonEmpty<T>`] + the distinct [`Verdict::Vacuous`] arm |
//! | **B. Unreached** | **not covered** — see above |
//! | **C. Discarded** | [`Permit<S>`] as a required argument (`#[must_use]` alone is provably insufficient — see [`permit`]) |
//! | **D. Degraded** | a closed four-arm sum, `Default = Unreached`, no `_ =>` arm anywhere in this crate |
//! | **E. Under-scoped** | the witness *is* the scanned set, and it is an output — though what set was scanned remains the caller's discipline |
//! | **F. Mirrored** | clause 1 only. `judge` derives the verdict from the subjects; nothing here can tell a derived set from a transcribed one |
//!
//! ## Red runs — recorded, per §II.3's standing rule
//!
//! A gate never observed to fail may be checking nothing, and reading the
//! source finds none of those (they all read correct) while running the gate
//! finds none of them either (they are all green). The one discipline that
//! catches it is to **deliberately break the guarded thing and watch for a
//! failure that does not come**. Four breaks were run against this crate on
//! 2026-07-28; each is reproducible from the description alone.
//!
//! | Break | Result |
//! |---|---|
//! | an `// EXPECT:` marker changed to name an error that does not fire | `compile_fail` FAILED, exit 101, naming the case and the missing substring |
//! | every file removed from `tests/ui/` | FAILED — *and `trybuild`'s own test PASSED on the empty corpus*, which is the defect the marker check exists to catch |
//! | a `tests/ui/` case that **does** compile | FAILED — both the `trybuild` case and the marker check went red |
//! | the wire re-derivation short-circuited to always accept | 4 forged-record tests FAILED |
//!
//! The second row is the interesting one and it was not hypothetical:
//! `trybuild` fails a case only when the case compiles, so an empty corpus
//! is a green run over zero subjects — §II.3's subclass A, appearing inside
//! this crate's own tests. Two `E0451` private-field claims were likewise
//! found suppressed by rustc behind an earlier error in the same file: the
//! files still failed to compile, the corpus still went green, and those two
//! guarantees were asserted rather than proven. Both are why every
//! compile-fail case now carries per-claim `// EXPECT:` markers checked
//! against the recorded stderr, and why the checker is itself routed through
//! [`Verdict`] so an empty corpus is [`Verdict::Vacuous`] rather than a pass.
//!
//! ## Cost
//!
//! Three dependencies: `serde`, `thiserror`, `gen-macros` (a proc-macro
//! crate whose output is self-contained inherent impls). That is deliberate.
//! A primitive that every gate, linter, reconciler observation, status field
//! and codegen backend in the fleet is supposed to adopt has to be cheap
//! enough that adopting it is never a dependency decision.

#![forbid(unsafe_code)]
#![allow(clippy::must_use_candidate)]

pub mod permit;
pub mod subjects;
pub mod verdict;

pub use permit::Permit;
pub use subjects::{Domain, EmptyWitness, Findings, NonEmpty, Subjects};
pub use verdict::{Verdict, VerdictError};
