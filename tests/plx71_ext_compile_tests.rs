//! PLX-71 trybuild test (acceptance criterion #4): a duplicate key inside a
//! single `#[method(ext(...))]` is a compile error whose message names the
//! duplicated key.
//!
//! Expected stderr lives next to the fixture and is refreshed via
//! `TRYBUILD=overwrite cargo test`.
//!
//! Run with: `cargo test --test plx71_ext_compile_tests`.

use trybuild::TestCases;

#[test]
fn plx71_ext_duplicate_key_compile_fail() {
    let t = TestCases::new();
    t.compile_fail("tests/compile/plx71_ext_duplicate_key.rs");
}
