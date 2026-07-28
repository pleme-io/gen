// EXPECT: field `subjects` of struct `Permit` is private
//
// The last door onto an unearned authorization. Isolated for the same
// suppression reason as `subjects_literal_is_private.rs`.

use gen_verdict::{Permit, Subjects};

fn main() {
    let _literal = Permit {
        subjects: Subjects::one(1_u8),
    };
}
