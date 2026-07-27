//! PLX-113 — the trybuild half: the §3.5 refusal of an empty root fact.
//!
//! Expected stderr lives next to the fixture and is refreshed with
//! `TRYBUILD=overwrite cargo test --test plx113_compile_tests`.

use trybuild::TestCases;

/// RFC 002 §3.5 — `backend_name = ""` / `respond_method = ""` is a compile
/// error, not a silently-normalized absence.
///
/// Omitting a root fact costs a SHOULD advisory; emitting it empty is a MUST
/// violation. The macro must not let an author trade the first for the second
/// by accident.
#[test]
fn an_empty_root_fact_is_a_compile_error_naming_3_5() {
    let t = TestCases::new();
    t.compile_fail("tests/compile/plx113_empty_root_fact.rs");
}
