//! MACRO-13 · the compile-time contract of the name rule, the trait, every
//! public method a wire method, and bodiless contracts (RETURNS-47: a trait
//! only when named).
//!
//! Expected stderr files live next to each negative fixture and are
//! refreshed with `TRYBUILD=overwrite cargo test --test macro13_compile_tests`.

use trybuild::TestCases;

#[test]
fn macro13_compile_pass() {
    let t = TestCases::new();
    // `trait = "Echo"` on `impl Server` builds `trait Echo` and `EchoClient`.
    t.pass("tests/compile/macro13_trait_named_on_any_type.rs");
}

#[test]
fn macro13_compile_fail() {
    let t = TestCases::new();
    // RETURNS-47 moved two cases to `returns47_compile_tests` as passes: a
    // type not named `…Activation` is named by itself
    // (`returns47_bare_type_names_itself`, was `macro13_cannot_name_the_trait`)
    // and nothing asks an item type for `FromClientError`
    // (`returns47_item_without_from_client_error`, was
    // `macro13_event_without_from_client_error`).
    // A `pub async fn` on `&self` returning `u32`, named.
    t.compile_fail("tests/compile/macro13_pub_async_non_stream.rs");
    // Bodied and bodiless wire methods, the bodiless one named.
    t.compile_fail("tests/compile/macro13_mixed_bodies.rs");
    // `client` under the name rule: the deprecation warning naming MACRO-13.
    t.compile_fail("tests/compile/macro13_client_flag_deprecated.rs");
    // `always`: an activation written before MACRO-13 builds, and is warned.
    t.compile_fail("tests/compile/macro13_unnamed_activation_deprecated.rs");
    // RETURNS-47: a bare `trait` names none.
    t.compile_fail("tests/compile/macro13_bare_trait_key.rs");
    // A contract method taking what the server injects, named.
    t.compile_fail("tests/compile/macro13_contract_injected.rs");
    // An activation that is not `Clone`, told so by name.
    t.compile_fail("tests/compile/macro13_activation_not_clone.rs");
}
