// EXPECT: named `default` found for struct `NonEmpty<T>`
// EXPECT: expected `NonEmpty<u8>`, found `Vec<u8>`
//
// §III.7's hardening #2, proven: with `head: T` as a real field there is no
// empty inhabitant to construct — not by `Default`, not by an infallible
// conversion from a `Vec`.
//
// Contrast with the checked-collection form §III.7 rejects, where
// `NonEmpty::try_from(vec![])` COMPILES and fails at runtime. The one
// fallible door here is `try_from_vec`, and it exists only because parse
// boundaries need one.
//
// The struct-literal door is proven separately in
// `subjects_literal_is_private.rs` — rustc suppresses E0451 when an earlier
// error in the same body has already fired, so keeping it here would leave a
// claim that looks proven and is not.

use gen_verdict::NonEmpty;

fn main() {
    let _defaulted: NonEmpty<u8> = NonEmpty::default();
    let _infallible: NonEmpty<u8> = NonEmpty::from(Vec::<u8>::new());
}
