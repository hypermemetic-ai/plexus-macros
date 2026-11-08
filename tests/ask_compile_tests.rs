//! ASK-2: what the macro refuses of `asks` — and `bidirectional`, which it
//! replaced. Run with `cargo test --test ask_compile_tests`; the expected
//! stderr sits beside each fixture (`TRYBUILD=overwrite` refreshes it).

use trybuild::TestCases;

#[test]
fn ask_compile_fail() {
    let t = TestCases::new();
    t.compile_fail("tests/compile/ask_without_confirm.rs");
    t.compile_fail("tests/compile/ask_confirm_without_asks.rs");
    t.compile_fail("tests/compile/ask_item_without_needs_input.rs");
    t.compile_fail("tests/compile/ask_bidirectional_removed.rs");
}
