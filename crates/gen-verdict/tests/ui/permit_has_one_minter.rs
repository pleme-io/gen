// EXPECT: associated function `mint` is private
// EXPECT: named `default` found for struct `Permit<S>`
// EXPECT: the trait bound `Permit<u8>: serde::Deserialize<'de>` is not satisfied
//
// The capability leg: a `Permit` is obtainable ONLY from a held verdict.
//
// This is what makes the permit stronger than `#[must_use]`. An attribute
// warns that a value went unused; a permit parameter means the guarded
// function cannot be called at all — and there is no second producer of the
// argument. Not a public mint, not a `Default`, not a deserialize.
//
// The struct-literal door is proven separately in
// `permit_literal_is_private.rs`, for the suppression reason recorded there.

use gen_verdict::{Permit, Subjects};

fn main() {
    let _minted = Permit::mint(Subjects::one(1_u8));
    let _defaulted: Permit<u8> = Permit::default();
    let _parsed: Permit<u8> = serde_json::from_str("[1]").unwrap();
}
