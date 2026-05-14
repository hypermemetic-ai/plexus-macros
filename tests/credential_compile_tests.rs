//! AUTHZ-CRED-MACRO-1 trybuild compile-pass and compile-fail fixtures for
//! the `#[plexus::credential(...)]` field attribute and its registry
//! emission.
//!
//! Run with: `cargo test --test credential_compile_tests`
//!
//! See `tests/compile/credential_*.rs` for the per-case fixtures.

use trybuild::TestCases;

#[test]
fn credential_compile_pass() {
    let t = TestCases::new();

    // Acceptance criteria 1, 4 — single bearer credential with all three
    // common clauses (kind, attach_as, scheme) + implicit sensitivity.
    t.pass("tests/compile/credential_basic_struct_ok.rs");

    // Acceptance criterion 2 — struct with multiple credential fields
    // produces one registry entry per field in declaration order.
    t.pass("tests/compile/credential_multi_field_struct_ok.rs");

    // Acceptance criterion 3 — enum with one variant carrying a credential
    // field produces one entry tagged with that variant.
    t.pass("tests/compile/credential_enum_variant_ok.rs");

    // Acceptance criterion 7 — a struct with no credential fields emits no
    // entries (and no errors). The macro is annotated on the struct as a
    // trigger, but no field carries `#[plexus::credential(...)]`.
    t.pass("tests/compile/credential_no_fields_ok.rs");

    // Acceptance criterion 8 (positive case) — the structured
    // `kind(other = "<name>")` escape valve compiles.
    t.pass("tests/compile/credential_kind_other_structured_ok.rs");
}

#[test]
fn credential_compile_fail() {
    let t = TestCases::new();

    // Acceptance criterion 5 — `#[plexus::credential(...)]` on a non-`Credential<T>`
    // field produces a compile error citing the wrapper requirement.
    t.compile_fail("tests/compile/credential_on_non_wrapper_field.rs");

    // Acceptance criterion 6 — missing `kind` clause.
    t.compile_fail("tests/compile/credential_missing_kind.rs");

    // Acceptance criterion 6 — missing `attach_as` clause.
    t.compile_fail("tests/compile/credential_missing_attach_as.rs");

    // Acceptance criterion 6 — unknown `kind` value.
    t.compile_fail("tests/compile/credential_unknown_kind.rs");

    // Acceptance criterion 6 — bad `attach_as` form.
    t.compile_fail("tests/compile/credential_bad_attach_as.rs");

    // Acceptance criterion 6 — bad `refresh_via` MethodPath.
    t.compile_fail("tests/compile/credential_bad_refresh_via.rs");

    // Acceptance criterion 6 — two `#[plexus::credential(...)]` attrs on the
    // same field.
    t.compile_fail("tests/compile/credential_duplicate_on_field.rs");

    // Acceptance criterion 8 (negative case) — the bare `kind = "other"`
    // form points users at the structured `kind(other = "<name>")` form.
    t.compile_fail("tests/compile/credential_kind_bare_other.rs");
}
