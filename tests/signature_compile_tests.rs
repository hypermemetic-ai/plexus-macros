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
    // `streaming`: cardinality comes from the return type.
    t.compile_fail("tests/compile/sig_streaming_removed.rs");
    // `returns(..)`: a method's own return type replaces the variant filter.
    t.compile_fail("tests/compile/sig_returns_removed.rs");
    // A boxed stream is not one of the six forms; the error lists them.
    t.compile_fail("tests/compile/sig_boxstream_rejected.rs");
    // Progress needs a stream: a one-item method cannot report it.
    t.compile_fail("tests/compile/sig_progress_needs_stream.rs");
}
