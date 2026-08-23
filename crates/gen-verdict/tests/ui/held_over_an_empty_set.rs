// EXPECT: cannot create non-exhaustive variant
// EXPECT: expected `NonEmpty<u8>`, found `Vec<u8>`
// EXPECT: named `empty` found for struct `NonEmpty<T>`
//
// THE headline proof: a pass over an empty subject set has no expressible
// form.
//
// Three attempts, three different reasons it cannot be written:
//
//   1. the `Held` arm cannot be NAMED downstream at all (`#[non_exhaustive]`),
//   2. even granted the arm, a `Vec` is not a `Subjects`,
//   3. and there is no empty `Subjects` to hand it in the first place.

use gen_verdict::{Subjects, Verdict};

fn main() {
    // 1. The arm is sealed downstream.
    let _sealed = Verdict::<u8, u8>::Held {
        subjects: Subjects::one(1),
    };

    // 2. Emptiness is not a `Subjects`.
    let _mistyped = Verdict::<u8, u8>::Held {
        subjects: Vec::<u8>::new(),
    };

    // 3. And there is no empty `Subjects` to reach for.
    let _no_empty_witness: Subjects<u8> = Subjects::empty();
}
