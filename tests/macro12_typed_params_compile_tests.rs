//! MACRO-12 · `#[plexus_macros::activation(typed_params)]`.
//!
//! Expected stderr lives beside the negative fixture; refresh with
//! `TRYBUILD=overwrite cargo test --test macro12_typed_params_compile_tests`.

use trybuild::TestCases;

#[test]
fn typed_params_compile_pass() {
    // Newtypes, enums and injected params compile under the flag; without
    // the flag nothing changes (every other fixture in tests/compile).
    TestCases::new().pass("tests/compile/macro12_typed_params_ok.rs");
}

#[test]
fn typed_params_compile_fail() {
    // String, std::string::String, &str, Option<String>, Option<&str>,
    // Vec<String>: each an error on its own parameter.
    TestCases::new().compile_fail("tests/compile/macro12_typed_params_refuses_strings.rs");
}
