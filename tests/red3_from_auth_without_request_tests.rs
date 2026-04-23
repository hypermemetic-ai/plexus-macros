//! RED-3 acceptance tests.

use trybuild::TestCases;

#[test]
fn from_auth_without_request_fails_to_compile() {
    let t = TestCases::new();
    t.compile_fail("tests/compile/red3_from_auth_without_request.rs");
}
