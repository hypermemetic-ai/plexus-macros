//! CHILD-3 + CHILD-4 compile-fail and compile-pass tests for the
//! `#[plexus_macros::child]` attribute.
//!
//! Positive fixtures must compile; negative fixtures must produce specific
//! compile errors. Expected stderr files live next to each negative fixture
//! and are generated / refreshed via `TRYBUILD=overwrite cargo test`.
//!
//! Run with: `cargo test --test child_compile_tests`.

use trybuild::TestCases;

#[test]
fn child_compile_pass() {
    let t = TestCases::new();
    t.pass("tests/compile/child_static_and_dynamic_ok.rs");
    // IR-20: generic activation with `#[child]` must compile — regression
    // for the synthesized `plugin_children()` impl block being non-generic.
    t.pass("tests/compile/ir20_generic_activation_with_child.rs");
    // MACRO-20: every `forward = "..."` spelling, and a hand-written
    // ChildRouter child beside them.
    t.pass("tests/compile/macro20_child_forward_ok.rs");
}

#[test]
fn child_compile_fail() {
    let t = TestCases::new();
    t.compile_fail("tests/compile/child_invalid_signature.rs");
    t.compile_fail("tests/compile/child_and_method_mutually_exclusive.rs");
    t.compile_fail("tests/compile/child_two_dynamic_methods.rs");
    t.compile_fail("tests/compile/child_and_legacy_children_list.rs");
    // CHILD-8
    t.compile_fail("tests/compile/child_and_hub_flag_ambiguous.rs");
    // CHILD-4
    t.compile_fail("tests/compile/child_list_not_found.rs");
    t.compile_fail("tests/compile/child_search_not_found.rs");
    t.compile_fail("tests/compile/child_list_wrong_signature.rs");
    t.compile_fail("tests/compile/child_search_wrong_signature.rs");
    // MACRO-20
    t.compile_fail("tests/compile/macro20_child_forward_unknown.rs");
}
