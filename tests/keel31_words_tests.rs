//! KEEL-31: an attribute word the macro does not know is a compile error
//! naming it and the words it takes — `#[method]`, `#[activation]`,
//! `#[child]` and `#[peer]` — so a word a newer macro introduced
//! (`everywhere`) cannot compile to nothing on an older one again.

#[test]
fn keel31_unknown_words_compile_fail() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile/keel31_unknown_method_word.rs");
    t.compile_fail("tests/compile/keel31_unknown_activation_word.rs");
    t.compile_fail("tests/compile/keel31_unknown_peer_word.rs");
}
