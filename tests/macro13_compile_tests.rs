//! MACRO-13 · the compile-time contract of the name rule, the trait, every
//! public method a wire method, and bodiless contracts.
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
    // A type the macro cannot name a trait for.
    t.compile_fail("tests/compile/macro13_cannot_name_the_trait.rs");
    // An event type without `FromClientError`, named.
    t.compile_fail("tests/compile/macro13_event_without_from_client_error.rs");
    // A `pub async fn` on `&self` returning `u32`, named.
    t.compile_fail("tests/compile/macro13_pub_async_non_stream.rs");
    // Bodied and bodiless wire methods, the bodiless one named.
    t.compile_fail("tests/compile/macro13_mixed_bodies.rs");
    // `client` under the name rule: the deprecation warning naming MACRO-13.
    t.compile_fail("tests/compile/macro13_client_flag_deprecated.rs");
    // `always`: an activation written before MACRO-13 builds, and is warned.
    t.compile_fail("tests/compile/macro13_unnamed_activation_deprecated.rs");
    // `always`: a bare `trait` asks for nothing.
    t.compile_fail("tests/compile/macro13_bare_trait_key.rs");
    // A contract method taking what the server injects, named.
    t.compile_fail("tests/compile/macro13_contract_injected.rs");
}
