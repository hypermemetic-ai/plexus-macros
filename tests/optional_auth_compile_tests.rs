//! AUTHZ-MACRO-OPTIONAL-AUTH-1 trybuild compile-pass driver.
//!
//! `tests/compile/optional_auth_ok.rs` declares two activations that
//! exercise the `auth: Option<&AuthContext>` handler shape (and, alongside
//! it, the legacy `auth: &AuthContext` shape) under `auth = "mixed"` and
//! `auth = "none"` postures. The trybuild assertion: both compile.

#[test]
fn optional_auth_handlers_compile() {
    let t = trybuild::TestCases::new();
    t.pass("tests/compile/optional_auth_ok.rs");
}
