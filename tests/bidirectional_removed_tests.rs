//! ASK-1: `bidirectional` (the in-call question) is removed, and nothing
//! replaces it: Plexus has no in-call or protocol-wide confirmation. Run with
//! `cargo test --test bidirectional_removed_tests`; the expected stderr sits
//! beside the fixture (`TRYBUILD=overwrite` refreshes it).

use trybuild::TestCases;

#[test]
fn bidirectional_is_a_compile_error() {
    let t = TestCases::new();
    t.compile_fail("tests/compile/bidirectional_removed.rs");
}
