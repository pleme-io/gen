// EXPECT: fields `head` and `tail` of struct `NonEmpty` are private
//
// The struct-literal door, closed — and isolated in its own file on purpose.
//
// §III.1's canonical example is graded `partially` precisely because its
// inner field is `pub(crate)` and gets literal-constructed in-crate, so
// holding the value does not prove the constructor ran. Both fields here are
// fully private.
//
// This case was originally written alongside other failures in one file and
// rustc SUPPRESSED it: the file still failed to compile, the corpus still
// went green, and this specific guarantee was being asserted rather than
// proven. That is the vacuous-guard shape appearing inside the vacuous-guard
// primitive's own tests, which is why every claim now carries an `EXPECT:`
// marker that `compile_fail.rs` checks against the recorded stderr.

use gen_verdict::NonEmpty;

fn main() {
    let _literal = NonEmpty {
        head: 1_u8,
        tail: Vec::new(),
    };
}
