//! The witness: a non-empty collection whose emptiness is a *compile*
//! error, and the sum that gives emptiness a name.
//!
//! [`NonEmpty<T>`] is the structural primitive `UNREPRESENTABILITY.md`
//! §III.7's hardening #2 prescribes verbatim — `{ head: T, tail: Vec<T> }`
//! — and which §III.7's own verdict records as shipping **nowhere** in the
//! fleet. This is the first one. [`Subjects<S>`] and [`Findings<F>`] are
//! role aliases over it, so §II.4's sketch reads verbatim at the use site
//! while the fleet gets one implementation rather than three.
//!
//! # Why head+tail rather than a checked `Vec`
//!
//! `NonEmpty::try_from(vec![])` compiles. That is the whole objection §III.7
//! raises against the checked-collection form: rejection is a runtime
//! `Result::Err`, so the type buys a *mitigation*, not an absence. With
//! `head: T` as a real field there is no empty inhabitant to name — an empty
//! `NonEmpty` is a type error, not a failed constructor.
//!
//! # What is deliberately absent
//!
//! No `DerefMut`. No `&mut Vec` accessor. No `pop`, `drain`, `clear`,
//! `truncate`, `retain`, `remove`. No derived `Default`. Each of those is a
//! path from a constructed non-empty value back to an empty one, which would
//! make the invariant hold at construction and nowhere else — §III.7's
//! second recorded gap.
//!
//! # The one runtime door
//!
//! [`NonEmpty::try_from_vec`] exists because bytes arrive from wires and
//! files as sequences, and a parse boundary is exactly where a
//! `Result::Err` is the honest answer (§III.1). It is the *only* fallible
//! ingress, and it is the one serde uses.

use core::num::NonZeroUsize;

use serde::{Deserialize, Serialize, Serializer};

/// A collection that is non-empty by construction.
///
/// The empty case has no inhabitant: `head` is a real field, so every value
/// of this type carries at least one `T`. Holding a `NonEmpty<T>` IS the
/// proof that at least one subject was examined — §III.2's proof-carrying
/// shape applied to a collection.
///
/// Fields are private and there is no public struct-literal path, so the
/// invariant cannot be skipped in-crate either (§III.1's canonical example
/// is graded `partially` precisely because its inner field is `pub(crate)`
/// and gets literal-constructed in-crate; this one does not).
///
/// # Serde
///
/// Serializes as a plain sequence — the wire shape of a `Vec<T>`, so no
/// consumer has to learn a new encoding. Deserializes through
/// [`NonEmpty::try_from_vec`], so an empty sequence is rejected at the parse
/// boundary rather than becoming a value that lies about itself.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
#[serde(try_from = "Vec<T>")]
pub struct NonEmpty<T> {
    head: T,
    tail: Vec<T>,
}

/// The subject set a verdict was derived from.
///
/// §II.4's name for [`NonEmpty`] in the verdict role: *the things that were
/// examined*.
pub type Subjects<S> = NonEmpty<S>;

/// The findings a falsified verdict carries.
///
/// §II.4's name for [`NonEmpty`] in the finding role. A `Falsified` arm with
/// zero findings would be a `Held` arm wearing the wrong tag, so the finding
/// set is non-empty by the same construction as the subject set.
pub type Findings<F> = NonEmpty<F>;

/// The result of scoping a candidate subject set.
///
/// A **sum**, never `Option`. §III.6's reason is the whole point: `Option`
/// invites `unwrap_or_default()`, which is precisely the silent round-up
/// this crate exists to remove. A named [`Domain::Empty`] arm forces the
/// caller to *say* what emptiness means in its domain — and the two shipped
/// instances say opposite things and are both right (magma's empty world
/// cannot drift, so it supports an in-sync claim; a linter that linted zero
/// files validated nothing, so it is an error).
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    gen_macros::Discriminant,
    gen_macros::IsVariant,
)]
#[discriminant(method = "kind", case = "snake")]
#[serde(rename_all = "snake_case")]
pub enum Domain<T> {
    /// At least one subject was in scope; the witness is attached.
    Populated(NonEmpty<T>),
    /// Zero subjects were in scope. Not a pass, not a failure — a distinct,
    /// nameable fact the caller must decide the meaning of.
    Empty,
}

/// The one failure mode of the one runtime door.
///
/// Reached only at a parse boundary ([`NonEmpty::try_from_vec`] and the
/// serde impl that calls it). In-Rust construction cannot produce it,
/// because in-Rust construction cannot express an empty `NonEmpty`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("an empty sequence cannot become a non-empty witness")]
pub struct EmptyWitness;

impl<T> NonEmpty<T> {
    /// The one-element witness. Compile-time non-emptiness: a `T` must be
    /// supplied, so there is nothing to check and nothing to fail.
    pub fn one(head: T) -> Self {
        Self {
            head,
            tail: Vec::new(),
        }
    }

    /// Build from a head and the rest.
    pub fn from_parts(head: T, tail: Vec<T>) -> Self {
        Self { head, tail }
    }

    /// Classify a candidate subject set. The **sum**, never `Option`.
    ///
    /// This is the ingress §II.4 names: a caller hands over whatever it
    /// examined and gets back a value that either carries a witness or says
    /// out loud that there was nothing to witness.
    pub fn scope(items: impl IntoIterator<Item = T>) -> Domain<T> {
        let mut it = items.into_iter();
        match it.next() {
            Some(head) => Domain::Populated(Self {
                head,
                tail: it.collect(),
            }),
            None => Domain::Empty,
        }
    }

    /// The one runtime door — for wire and file boundaries only.
    ///
    /// # Errors
    ///
    /// [`EmptyWitness`] when `items` is empty.
    pub fn try_from_vec(items: Vec<T>) -> Result<Self, EmptyWitness> {
        let mut it = items.into_iter();
        let head = it.next().ok_or(EmptyWitness)?;
        Ok(Self {
            head,
            tail: it.collect(),
        })
    }

    /// How many subjects the witness carries.
    ///
    /// A **projection of the value**, never a field a caller sets. The
    /// distinction is the whole of §II.4's first bullet: a `usize` field is
    /// assertable, and even a `NonZeroUsize` field only fixes zero while
    /// carrying no provenance. Here the count and the claim share one
    /// origin, because the only way to obtain the count is to hold the
    /// subjects.
    #[must_use]
    pub fn count(&self) -> NonZeroUsize {
        match NonZeroUsize::new(self.tail.len().saturating_add(1)) {
            Some(n) => n,
            // `tail.len() + 1 >= 1` for every representable `tail`, so this
            // arm is uninhabited by arithmetic. Kept total rather than
            // panicking: a witness type that can panic while reporting its
            // own size would be a poor advertisement.
            None => NonZeroUsize::MIN,
        }
    }

    /// The first subject. Always present — that is the point of the type.
    pub fn first(&self) -> &T {
        &self.head
    }

    /// Borrow every subject in order.
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        core::iter::once(&self.head).chain(self.tail.iter())
    }

    /// Consume the witness into a plain `Vec`.
    ///
    /// One-way on purpose: the resulting `Vec` no longer proves anything,
    /// and there is no total function back.
    #[must_use]
    pub fn into_vec(self) -> Vec<T> {
        let mut out = Vec::with_capacity(self.tail.len().saturating_add(1));
        out.push(self.head);
        out.extend(self.tail);
        out
    }

    /// Add a subject. Growing a non-empty set keeps it non-empty, so this is
    /// the one mutation that cannot violate the invariant — and the reason
    /// `pop` has no counterpart here.
    pub fn push(&mut self, item: T) {
        self.tail.push(item);
    }

    /// Map every subject, preserving non-emptiness.
    pub fn map<U>(self, mut f: impl FnMut(T) -> U) -> NonEmpty<U> {
        NonEmpty {
            head: f(self.head),
            tail: self.tail.into_iter().map(f).collect(),
        }
    }
}

impl<T> TryFrom<Vec<T>> for NonEmpty<T> {
    type Error = EmptyWitness;

    fn try_from(items: Vec<T>) -> Result<Self, Self::Error> {
        Self::try_from_vec(items)
    }
}

impl<T> From<NonEmpty<T>> for Vec<T> {
    fn from(value: NonEmpty<T>) -> Self {
        value.into_vec()
    }
}

impl<'a, T> IntoIterator for &'a NonEmpty<T> {
    type Item = &'a T;
    type IntoIter = core::iter::Chain<core::iter::Once<&'a T>, core::slice::Iter<'a, T>>;

    fn into_iter(self) -> Self::IntoIter {
        core::iter::once(&self.head).chain(self.tail.iter())
    }
}

impl<T> IntoIterator for NonEmpty<T> {
    type Item = T;
    type IntoIter = std::vec::IntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        self.into_vec().into_iter()
    }
}

// Hand-written rather than `#[serde(into = "Vec<T>")]`: the container
// attribute would impose `T: Clone` on every consumer that merely wants to
// serialize a witness, which is a real cost for zero benefit.
impl<T: Serialize> Serialize for NonEmpty<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.iter())
    }
}

impl<T> Domain<T> {
    /// The subject count in scope — zero for [`Domain::Empty`].
    ///
    /// Reporting only. A caller that wants to *act* on the subjects must
    /// match on the arm and take the witness, which is what keeps the empty
    /// case from silently becoming a pass.
    #[must_use]
    pub fn count(&self) -> usize {
        match self {
            Self::Populated(s) => s.count().get(),
            Self::Empty => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_of_an_empty_iterator_is_the_empty_arm_not_a_witness() {
        let d: Domain<u8> = NonEmpty::scope(Vec::<u8>::new());
        assert!(d.is_empty());
        assert_eq!(d.count(), 0);
    }

    #[test]
    fn scope_of_a_populated_iterator_carries_every_subject() {
        let Domain::Populated(s) = NonEmpty::scope(vec![7, 8, 9]) else {
            panic!("three subjects must scope to Populated");
        };
        assert_eq!(s.count().get(), 3);
        assert_eq!(s.first(), &7);
        assert_eq!(s.iter().copied().collect::<Vec<_>>(), vec![7, 8, 9]);
    }

    #[test]
    fn one_is_non_empty_without_a_check() {
        let s = NonEmpty::one("only");
        assert_eq!(s.count().get(), 1);
    }

    #[test]
    fn the_runtime_door_rejects_an_empty_vec() {
        assert_eq!(NonEmpty::<u8>::try_from_vec(Vec::new()), Err(EmptyWitness));
    }

    #[test]
    fn push_grows_the_witness_and_there_is_no_pop() {
        let mut s = NonEmpty::one(1);
        s.push(2);
        assert_eq!(s.count().get(), 2);
    }

    #[test]
    fn a_witness_round_trips_as_a_plain_sequence() {
        let s = NonEmpty::from_parts(1_u8, vec![2, 3]);
        let json = serde_json::to_string(&s).expect("serialize");
        assert_eq!(json, "[1,2,3]");
        let back: NonEmpty<u8> = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, s);
    }

    #[test]
    fn an_empty_sequence_is_rejected_at_the_parse_boundary() {
        let err = serde_json::from_str::<NonEmpty<u8>>("[]").expect_err("empty must not parse");
        assert!(
            err.to_string().contains("non-empty witness"),
            "the rejection must say what was missing: {err}",
        );
    }
}
