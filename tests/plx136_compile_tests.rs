//! PLX-136 (ACP·B) c2 — **the second source of truth does not exist.**
//!
//! c2 asks for one of two proofs: that there is no input which advertises a
//! capability without the impl, or a compile-fail test if a declaration
//! surface exists at all. Both are given here, because a declaration surface
//! does exist in the weak sense that `#[acp_agent(...)]` takes *some* keys:
//!
//! - **No such input.** `acp_agent::ACP_AGENT_KEYS` is `acp_path`, `name`,
//!   `version` — the whole surface. None of them names a capability, and
//!   `name`/`version` populate `agentInfo`, which is identity. The one item
//!   that could return a hand-written `AgentCapabilities` is `initialize`,
//!   which the macro generates and which an author may not write.
//! - **Compile-fail tests.** Every spelling of the missing surface someone
//!   would plausibly reach for is pinned below, with an error that names the
//!   impl to write instead. PLX-93/PLX-147 made unknown keys an error on the
//!   `#[method]` and `#[activation]` paths; this holds the same property on
//!   the third parser, so a typo is caught rather than declaring nothing.
//!
//! Regenerate the `.stderr` pins with:
//! `TRYBUILD=overwrite cargo test --test plx136_compile_tests`

use trybuild::TestCases;

/// c2 — `AgentCapabilities` cannot be declared as a whole.
#[test]
fn a_capabilities_key_does_not_exist() {
    let t = TestCases::new();
    t.compile_fail("tests/compile/plx136_capabilities_key.rs");
}

/// c2 — nor one capability at a time.
#[test]
fn a_per_capability_key_does_not_exist() {
    let t = TestCases::new();
    t.compile_fail("tests/compile/plx136_load_session_key.rs");
}

/// c2, advertisement site 3 — `authMethods` is not declarable at initialize
/// time; it follows from implementing `authenticate`.
#[test]
fn an_auth_methods_key_does_not_exist() {
    let t = TestCases::new();
    t.compile_fail("tests/compile/plx136_auth_methods_key.rs");
}

/// c2, advertisement site 2 — the pair PLX-135 warned would be dropped. There
/// is no initialize-time key for them because there is no initialize-time
/// FIELD for them; they belong to the session.
#[test]
fn a_per_session_key_does_not_exist_because_the_site_is_the_session() {
    let t = TestCases::new();
    t.compile_fail("tests/compile/plx136_modes_key.rs");
}

/// Required behaviour 4 — `ext(...)` does not survive as a mapping mechanism.
#[test]
fn ext_is_not_a_mapping_mechanism_for_acp() {
    let t = TestCases::new();
    t.compile_fail("tests/compile/plx136_ext_key.rs");
}

/// c2 — the last declaration surface: a method body that returns an
/// `InitializeResponse`. `initialize` is generated, not authored.
#[test]
fn an_author_written_initialize_is_the_second_source_of_truth_and_is_refused() {
    let t = TestCases::new();
    t.compile_fail("tests/compile/plx136_author_initialize.rs");
}

/// c2, the strongest form — the surface `#[acp_agent]` does NOT own: writing
/// the `Agent` impl yourself, by hand, with a hand-built `AgentCapabilities`.
/// `E0119`: the generated impl is the only impl.
#[test]
fn a_hand_written_second_agent_impl_cannot_coexist_with_the_generated_one() {
    let t = TestCases::new();
    t.compile_fail("tests/compile/plx136_second_agent_impl.rs");
}

/// The trait's own errors, which is why PLX-76 chose a trait: a missing
/// required item enumerates the surface.
#[test]
fn a_missing_required_item_is_e0046() {
    let t = TestCases::new();
    t.compile_fail("tests/compile/plx136_missing_required_item.rs");
}

/// …and a misspelled one is E0407 rather than a method that expands fine and
/// fails at runtime against a live editor.
#[test]
fn a_misspelled_item_is_e0407() {
    let t = TestCases::new();
    t.compile_fail("tests/compile/plx136_misspelled_item.rs");
}

/// Advertisement site 3, the compile-time half: implementing `authenticate`
/// makes `acp_auth_methods` mandatory, so the obligation cannot be forgotten.
#[test]
fn implementing_authenticate_makes_auth_methods_a_compile_time_obligation() {
    let t = TestCases::new();
    t.compile_fail("tests/compile/plx136_authenticate_without_auth_methods.rs");
}

/// PLX-93 / PLX-147's property, on the third parser.
#[test]
fn an_unknown_key_is_an_error_naming_the_whole_accepted_surface() {
    let t = TestCases::new();
    t.compile_fail("tests/compile/plx136_unknown_key.rs");
}

/// c3 — the floor case compiles, and is object-safe.
#[test]
fn the_floor_case_compiles() {
    let t = TestCases::new();
    t.pass("tests/compile/plx136_floor_ok.rs");
}
