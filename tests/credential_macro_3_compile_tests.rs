//! AUTHZ-CRED-MACRO-3 trybuild fixtures for the wired path:
//! `#[plexus_macros::activation]` threads the macro-emitted
//! `CredentialFieldMarker` registry through
//! `CredentialsRegistryProbe` and into `MethodSchema.credentials`.
//!
//! Run with: `cargo test --test credential_macro_3_compile_tests`
//!
//! See `tests/compile/credential_macro_3_*.rs` for the per-case fixtures.

use trybuild::TestCases;

#[test]
fn credential_macro_3_compile_pass() {
    let t = TestCases::new();

    // AC #1 + AC #4: method whose return type derives `Credentials` produces
    // a `MethodSchema` with populated `credentials` field. Demonstrates the
    // wired path end to end.
    t.pass("tests/compile/credential_macro_3_wired_ok.rs");

    // AC #2: method whose return type does NOT derive `Credentials` produces
    // a `MethodSchema` with `credentials: []` (unchanged behavior). Confirms
    // the autoref-blanket fallback fires for plain types.
    t.pass("tests/compile/credential_macro_3_plain_return_ok.rs");

    // AC #5: `#[derive(Credentials)]` emits the `HasCredentialMarkers`
    // trait impl that wires the autoref-specialized path. The trait impl
    // and the legacy per-type marker function return identical slices.
    t.pass("tests/compile/credential_macro_3_derive_emits_trait_ok.rs");
}
