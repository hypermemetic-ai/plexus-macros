//! RETURNS-8 · the signature is the contract. The attributes it replaces and
//! a return type outside the six signature forms are compile errors that say
//! what to write instead.
//!
//! Expected stderr files live next to each fixture; refresh them with
//! `TRYBUILD=overwrite cargo test --test signature_compile_tests`.

use trybuild::TestCases;

#[test]
fn signature_compile_fail() {
    let t = TestCases::new();
    // `streaming` where the signature disagrees (a one-item form).
    t.compile_fail("tests/compile/sig_streaming_on_one_item.rs");
    // `streaming` where it agrees: a deprecation warning (denied here).
    t.compile_fail("tests/compile/sig_streaming_deprecated.rs");
    // `returns(..)`: a method's own return type replaces the variant filter.
    t.compile_fail("tests/compile/sig_returns_removed.rs");
    // A boxed stream is not one of the six forms; the error lists them.
    t.compile_fail("tests/compile/sig_boxstream_rejected.rs");
    // Progress needs a stream: a one-item method cannot report it.
    t.compile_fail("tests/compile/sig_progress_needs_stream.rs");
}

#[test]
fn signature_compile_pass() {
    let t = TestCases::new();
    // RETURNS-36: `streaming` on a stream form is a deprecated no-op.
    t.pass("tests/compile/sig_streaming_agrees_ok.rs");
}
