// EXPECT: cannot create non-exhaustive variant
//
// Clause 1, downstream: `judge` is not the recommended ingress, it is the
// only one. `Falsified` is sealed for the same reason `Held` is — a caller
// that can name the arm can pair any subject set with any findings, which is
// §II.3's subclass F (a well-typed expectation and a well-typed subject set,
// both asserted rather than derived).

use gen_verdict::{Findings, Subjects, Verdict};

fn main() {
    let _sealed = Verdict::<u8, u8>::Falsified {
        subjects: Subjects::one(1),
        findings: Findings::one(9),
    };
}
