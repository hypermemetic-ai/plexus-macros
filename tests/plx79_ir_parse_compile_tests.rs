//! PLX-79 trybuild tests (acceptance criterion #3): the vNext parser rejects
//! unknown `#[method(...)]` keys, naming the key, and rejects a capability
//! handle that does not name its capability set.
//!
//! Expected stderr lives next to each fixture and is refreshed via
//! `TRYBUILD=overwrite cargo test`.
//!
//! Run with: `cargo test --test plx79_ir_parse_compile_tests`.

use trybuild::TestCases;

#[test]
fn plx79_unknown_method_key_compile_fail() {
    let t = TestCases::new();
    t.compile_fail("tests/compile/plx79_unknown_method_key.rs");
}

#[test]
fn plx79_handle_without_capability_set_compile_fail() {
    let t = TestCases::new();
    t.compile_fail("tests/compile/plx79_handle_without_set.rs");
}
