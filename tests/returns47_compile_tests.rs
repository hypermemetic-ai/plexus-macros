//! RETURNS-47 · the compile-time half: what the end of the suffix rule, the
//! trait only when named and `impl Trait for Type` refuse.
//!
//! Expected stderr files live next to each fixture and are refreshed with
//! `TRYBUILD=overwrite cargo test --test returns47_compile_tests`.

use trybuild::TestCases;

#[test]
fn returns47_compile_pass() {
    let t = TestCases::new();
    // A type with no suffix and nothing in the attribute: its own name names it.
    t.pass("tests/compile/returns47_bare_type_names_itself.rs");
    // An event type with no `FromClientError`: nothing asks for it.
    t.pass("tests/compile/returns47_item_without_from_client_error.rs");
}

#[test]
fn returns47_compile_fail() {
    let t = TestCases::new();
    // The client implements no trait, a named one included.
    t.compile_fail("tests/compile/returns47_client_implements_no_trait.rs");
    // `trait = "…"` on `impl Trait for Type`.
    t.compile_fail("tests/compile/returns47_trait_key_on_trait_impl.rs");
    // A bodiless `impl Driver` with no trait named.
    t.compile_fail("tests/compile/returns47_bare_contract_unnamed.rs");
}
