// EXPECT: no method named `unwrap_or_default`
// EXPECT: no method named `unwrap`
// EXPECT: no method named `unwrap_or`
//
// §III.6, proven: scoping returns a SUM, so the `Option` escape hatches that
// silently round an empty domain up to a usable value do not exist.
//
// `unwrap_or_default()` is the specific move this design exists to remove —
// it is how an empty scope becomes a pass without anyone writing the word
// "pass".

use gen_verdict::Subjects;

fn main() {
    let scoped = Subjects::scope(Vec::<u8>::new());

    let _rounded_up = scoped.unwrap_or_default();
    let _unwrapped = Subjects::scope(Vec::<u8>::new()).unwrap();
    let _defaulted = Subjects::scope(Vec::<u8>::new()).unwrap_or(Subjects::one(0));
}
