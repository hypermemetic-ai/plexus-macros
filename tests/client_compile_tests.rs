//! MACRO-2 · compile-time contract of the `client` / `server_cfg` flags.
//!
//! Expected stderr files live next to each negative fixture and are
//! refreshed with `TRYBUILD=overwrite cargo test --test client_compile_tests`.

use trybuild::TestCases;

#[test]
fn client_compile_pass() {
    let t = TestCases::new();
    // An activation that does not opt in is untouched by this build.
    t.pass("tests/compile/client_opt_out_is_unchanged.rs");
}

#[test]
fn client_compile_fail() {
    let t = TestCases::new();
    // `server_cfg` with nothing to keep is a mistake, not an empty expansion.
    t.compile_fail("tests/compile/client_server_cfg_without_client.rs");
    // (RETURNS-8 retired `client_non_stream_method`: every signature form now
    // has an item, so every method is on the client — the conformance
    // fixture, `fixtures/conformance`, compiles all six with `client`.)
}
