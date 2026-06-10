//! R-1 trybuild tests (AUTHZ-CORE-2 acceptance criteria #9 + #10): the
//! `public` + `scope = "..."` contradiction and invalid scope literals are
//! compile errors; valid scope/public/posture combinations compile.
//!
//! Expected stderr files live next to each negative fixture and are
//! refreshed via `TRYBUILD=overwrite cargo test`.
//!
//! Run with: `cargo test --test authz_scope_compile_tests`.

use trybuild::TestCases;

#[test]
fn authz_scope_compile_pass() {
    let t = TestCases::new();
    t.pass("tests/compile/authz_scope_ok.rs");
}

#[test]
fn authz_scope_compile_fail() {
    let t = TestCases::new();
    // AC #9: a public method has no required scope to enforce.
    t.compile_fail("tests/compile/authz_public_scope_conflict.rs");
    // AC #10: scope literals must satisfy the AUTHZ-CORE-4 grammar.
    t.compile_fail("tests/compile/authz_invalid_scope_literal.rs");
    t.compile_fail("tests/compile/authz_scope_not_a_string.rs");
}
