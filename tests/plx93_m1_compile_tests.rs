//! PLX-93 (M1·I) — the trybuild half of the M1 coverage matrix.
//!
//! Two T-IDs from PLX-73's reference suite had no compile-fail coverage on the
//! path that ships. Each driver below names the T-ID it discharges; the
//! fixtures carry the argument.
//!
//! Expected stderr lives next to each fixture and is refreshed with
//! `TRYBUILD=overwrite cargo test --test plx93_m1_compile_tests`.

use trybuild::TestCases;

/// **M1-T6** — `#[method(unknwon = 1)]` is a compile error that names the key,
/// on `#[activation]` and not only on `method_ir!`.
///
/// Before PLX-93 this fixture COMPILED. `Mode::Legacy` had no allowlist, so
/// PLX-73's trap closure was installed on the standalone parser macro and
/// absent from every live service. See `LEGACY_ONLY_KEYS` in
/// `src/ir_parse/attrs.rs`.
#[test]
fn m1_t6_unknown_method_key_on_a_real_activation_is_a_compile_error() {
    let t = TestCases::new();
    t.compile_fail("tests/compile/plx93_activation_unknown_method_key.rs");
}

/// **M1-T7 (amended)** — the surface M1 actually removed fails loud.
///
/// The credentials derive and the 0.5 `StreamEvent` derive were deleted by
/// PLX-92 with zero live consumers and no test. `scope`, `public` and
/// `bidirectional` were deliberately NOT removed; the fixture's header records
/// which decision spared each one.
#[test]
fn m1_t7_the_removed_derives_no_longer_resolve() {
    let t = TestCases::new();
    t.compile_fail("tests/compile/plx93_removed_derives_are_gone.rs");
}
