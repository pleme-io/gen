// EXPECT: no method named `pop`
// EXPECT: no method named `clear`
// EXPECT: no method named `drain`
// EXPECT: no method named `truncate`
// EXPECT: no method named `remove`
// EXPECT: no method named `retain`
// EXPECT: no method named `as_mut_vec`
//
// A non-empty type whose invariant holds only at construction is §III.7's
// second recorded gap: a `DerefMut`/`&mut Vec` accessor drains it back to
// empty afterwards.
//
// Every shrinking door is absent. `push` (growing) is present, because
// growing a non-empty set cannot violate the invariant — which is exactly
// why it has no counterpart.

use gen_verdict::NonEmpty;

fn main() {
    let mut subjects = NonEmpty::from_parts(1_u8, vec![2, 3]);

    let _ = subjects.pop();
    subjects.clear();
    let _ = subjects.drain(..);
    subjects.truncate(0);
    subjects.remove(0);
    subjects.retain(|_| false);
    subjects.as_mut_vec().clear();
}
