//! PERF.115-43 · `#[derive(plexus_macros::Deserialize)]`: what it builds, and
//! what it refuses, by name, at compile time.
//!
//! Expected stderr files live next to each negative fixture and are
//! refreshed with `TRYBUILD=overwrite cargo test --test type_first_compile_tests`.

use trybuild::TestCases;

#[test]
fn type_first_compile_pass() {
    let t = TestCases::new();
    // A house event, read with its type first and with it last.
    t.pass("tests/compile/type_first_reads.rs");
}

#[test]
fn type_first_compile_fail() {
    let t = TestCases::new();
    // A struct, an enum with no tag, an adjacently tagged, an untagged and a
    // generic enum, and one with an untagged variant: each told to derive
    // serde's `Deserialize`.
    t.compile_fail("tests/compile/type_first_refused.rs");
}
