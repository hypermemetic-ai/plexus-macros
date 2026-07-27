//! PLX-147 — the trybuild half: `#[activation(...)]` refuses what it used to
//! drop.
//!
//! PLX-93 made an unrecognized `#[method(...)]` key an error in both strict and
//! legacy modes. One level up the trap stayed open: `HubMethodsAttrs::parse`
//! matched known keys and ignored everything else, so PLX-113's probe
//! (`#[activation(bakcend_name = "oops", respnod_method = "oops",
//! totally_made_up = "oops")]`) compiled clean and dropped all three.
//!
//! Expected stderr lives next to each fixture and is refreshed with
//! `TRYBUILD=overwrite cargo test --test plx147_compile_tests`.

use trybuild::TestCases;

/// A typo'd key is named, and the accepted set is listed.
#[test]
fn an_unknown_activation_key_is_a_compile_error_naming_the_key() {
    let t = TestCases::new();
    t.compile_fail("tests/compile/plx147_unknown_activation_key.rs");
}

/// The live casualty's spelling gets a tailored error pointing at `auth`.
#[test]
fn the_auth_posture_misspelling_is_refused_by_name() {
    let t = TestCases::new();
    t.compile_fail("tests/compile/plx147_auth_posture_typo.rs");
}

/// A recognized key in an unreadable form is refused too — it was dropped just
/// as silently as an unknown one.
#[test]
fn a_recognized_key_in_the_wrong_form_is_a_compile_error() {
    let t = TestCases::new();
    t.compile_fail("tests/compile/plx147_wrong_form_activation_key.rs");
}

// ===========================================================================
// PLX-160 — a `#[child]` argument the emitter cannot honour
// ===========================================================================

/// `list = "…"` naming a method the impl block does not define.
///
/// This became load-bearing with PLX-160: before it, `list =` on a *dynamic*
/// child was discarded before reaching the IR, so a typo had no diagnostic and
/// no effect. Now it is written to the wire as RFC 002 §5.1's mandatory
/// enumeration method, and an unresolvable one advertises a method the backend
/// does not have — the fabrication §5.2 forbids.
#[test]
fn a_list_method_the_impl_block_does_not_define_is_a_compile_error() {
    let t = TestCases::new();
    t.compile_fail("tests/compile/plx160_list_names_nothing.rs");
}
