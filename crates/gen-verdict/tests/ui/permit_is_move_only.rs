// EXPECT: no method named `clone`
// EXPECT: use of moved value: `permit`
//
// One earned authorization authorizes one action.
//
// `authorize` takes `self`, and `Permit` is neither `Copy` nor `Clone`, so a
// permit cannot be spent twice and cannot be duplicated to cover a second
// subject set.

use gen_verdict::{Subjects, Verdict};

fn main() {
    let permit = Verdict::<u8, u8>::judge(Subjects::scope(vec![1, 2]), Vec::new())
        .into_permit()
        .expect("held");

    let _first = permit.authorize(|examined| examined.count());
    // Spent.
    let _second = permit.authorize(|examined| examined.count());
}

fn duplicated() {
    let permit = Verdict::<u8, u8>::judge(Subjects::scope(vec![1, 2]), Vec::new())
        .into_permit()
        .expect("held");
    let _copy = permit.clone();
}
