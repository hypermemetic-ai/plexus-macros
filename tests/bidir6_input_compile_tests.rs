//! BIDIR-6: what the macro refuses of an `Input`. Run with
//! `cargo test --test bidir6_input_compile_tests`; the expected stderr sits
//! beside each fixture (`TRYBUILD=overwrite` refreshes it).

use trybuild::TestCases;

#[test]
fn bidir6_input_compile_fail() {
    let t = TestCases::new();
    t.compile_fail("tests/compile/bidir6_two_inputs.rs");
    t.compile_fail("tests/compile/bidir6_nested_input.rs");
    t.compile_fail("tests/compile/bidir6_input_in_a_struct.rs");
    t.compile_fail("tests/compile/bidir6_window_without_input.rs");
    t.compile_fail("tests/compile/bidir6_window_of_nothing.rs");
}
