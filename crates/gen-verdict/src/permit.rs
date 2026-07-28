//! The permit: a move-only capability minted only by a held verdict.
//!
//! # Why `#[must_use]` is not enough
//!
//! `#[must_use]` is a **warning about an unused value**. It says nothing
//! about what the value was derived from, and nothing about whether the
//! guarded action was reachable without it. The fleet has a shipped receipt
//! that it is insufficient on its own: `sui/sui-spec/src/parity.rs:359-362`
//! carries `#[must_use]` on
//!
//! ```ignore
//! pub fn all_pass(&self) -> bool { self.records.iter().all(|r| r.verdict.is_pass()) }
//! ```
//!
//! — and returns `true` over an empty `records`, so the attribute is
//! faithfully enforced on a verdict that is vacuous. The attribute was never
//! the defence; it just made sure nobody ignored the wrong answer.
//!
//! A [`Permit<S>`] is a different mechanism. A guarded action declares
//! `fn apply(permit: Permit<Change>)`, so it **cannot be called at all**
//! without a value that only [`Verdict::Held`](crate::Verdict::Held) mints.
//! That is an argument the compiler requires, not a lint the compiler
//! suggests.
//!
//! # The binding — and the honest residual, stated on day one
//!
//! The fleet's named hole in the capability-token pattern is that *the
//! witness does not bind to the payload*: a permit earned over subject set
//! A could authorize action on set B.
//!
//! This permit **binds it on the input side**. `Permit<S>` owns the
//! `Subjects<S>` it was minted from, and [`Permit::authorize`] consumes the
//! permit by value and hands that owned witness to the action. The action's
//! input therefore *is* the examined set — there is no path by which a
//! caller obtains a permit over A and is handed B.
//!
//! **What is not bound, and cannot be in Rust:** the action's *behaviour*.
//! `permit.authorize(|_examined| act_on_something_else())` compiles. Rust
//! has no way to require that a closure consult its argument. So:
//!
//! | Axis | Tier |
//! |---|---|
//! | the guarded action is reachable without a held verdict | **truly-unrepresentable** — the parameter has no other producer |
//! | the action is *handed* a set other than the examined one | **truly-unrepresentable** — `authorize` moves the permit's own witness in |
//! | the action *operates on* the set it was handed | **only-mitigated** — no type expresses "this argument must be used" |
//!
//! The third row is the residual, and it is smaller than the fleet's named
//! one: the gap is no longer "a permit for A authorized work on B", it is
//! "an action was handed A and chose to ignore it", which is visible in the
//! action's own signature and body rather than hidden across two call sites.
//! It is not zero, and this crate does not grade it as zero.

use core::num::NonZeroUsize;

use crate::subjects::Subjects;

/// Proof that a [`Verdict::Held`](crate::Verdict::Held) authorized this
/// action, carrying the subject set it was earned over.
///
/// No public constructor, no public field, no `Default`, no `Clone`, no
/// `Copy`, no `Serialize`. The sole minter is
/// [`Verdict::into_permit`](crate::Verdict::into_permit); the sole
/// consumption is by value. A permit cannot be duplicated to authorize a
/// second action, and cannot be reconstructed from bytes.
#[derive(Debug)]
#[must_use = "a Permit is an authorization earned from a held verdict; \
              dropping it without authorizing anything discards the proof"]
pub struct Permit<S> {
    subjects: Subjects<S>,
}

impl<S> Permit<S> {
    /// The crate-private mint. Called from exactly one place —
    /// [`Verdict::into_permit`](crate::Verdict::into_permit) — and reachable
    /// from nowhere outside this crate.
    pub(crate) fn mint(subjects: Subjects<S>) -> Self {
        Self { subjects }
    }

    /// How many subjects this authorization was earned over. Never zero:
    /// the witness is a [`Subjects`], which has no empty inhabitant.
    #[must_use]
    pub fn count(&self) -> NonZeroUsize {
        self.subjects.count()
    }

    /// Borrow the authorized subject set — for logging and receipts.
    pub fn subjects(&self) -> &Subjects<S> {
        &self.subjects
    }

    /// Consume the permit and run the guarded action **over the examined
    /// set**.
    ///
    /// The move is the point twice over: the permit cannot authorize a
    /// second action, and the action's input is the very witness the verdict
    /// was derived from.
    pub fn authorize<T>(self, action: impl FnOnce(Subjects<S>) -> T) -> T {
        action(self.subjects)
    }

    /// Consume the permit for its witness, without a closure.
    ///
    /// Equivalent to [`Permit::authorize`] with an identity action; provided
    /// for call sites where the guarded work is not a single expression.
    #[must_use]
    pub fn into_subjects(self) -> Subjects<S> {
        self.subjects
    }
}

#[cfg(test)]
mod tests {
    use crate::{NonEmpty, Verdict};

    fn judged(subjects: Vec<u8>, findings: Vec<&'static str>) -> Verdict<u8, &'static str> {
        Verdict::judge(NonEmpty::scope(subjects), findings)
    }

    #[test]
    fn a_held_verdict_mints_a_permit_carrying_its_witness() {
        let permit = judged(vec![4, 5], Vec::new())
            .into_permit()
            .expect("a held verdict mints");
        assert_eq!(permit.count().get(), 2);
        let seen = permit.authorize(|s| s.iter().copied().collect::<Vec<_>>());
        assert_eq!(seen, vec![4, 5], "the action is handed the examined set");
    }

    #[test]
    fn a_vacuous_verdict_mints_nothing() {
        let refused = judged(Vec::new(), Vec::new())
            .into_permit()
            .expect_err("nothing examined, nothing authorized");
        assert_eq!(refused, Verdict::Vacuous);
    }

    #[test]
    fn a_falsified_verdict_mints_nothing_and_keeps_its_reason() {
        let refused = judged(vec![1], vec!["bad"])
            .into_permit()
            .expect_err("findings deny the permit");
        assert!(refused.is_falsified());
        assert_eq!(refused.findings().map(|f| f.count().get()), Some(1));
    }

    #[test]
    fn an_unreached_verdict_mints_nothing() {
        let refused = Verdict::<u8, &str>::default()
            .into_permit()
            .expect_err("a check that never ran authorizes nothing");
        assert_eq!(refused, Verdict::Unreached);
    }
}
