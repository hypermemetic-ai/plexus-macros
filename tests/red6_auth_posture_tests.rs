//! RED-6 acceptance tests: activation `auth = "..."` posture enforcement.
//!
//! Positive paths (compile cleanly) live here; the trybuild-driven
//! compile-fail fixtures live in tests/compile/red6_*.rs + .stderr.

use plexus_core::plexus::{Activation, AuthContext, PlexusError};
use plexus_macros::PlexusRequest;

#[derive(Clone, Debug)]
struct FakeUser;

#[derive(PlexusRequest)]
struct Req {
    #[from_cookie("access_token")]
    auth_token: String,
}

// ─── auth = "required", every method is gated ────────────────────

struct Strict;

impl Strict {
    #[allow(dead_code)]
    async fn validate(&self, _: &AuthContext) -> Result<FakeUser, PlexusError> { Ok(FakeUser) }
}

#[plexus_macros::activation(
    namespace = "strict",
    request = Req,
    auth = "required",
    crate_path = "plexus_core",
)]
impl Strict {
    #[plexus_macros::method]
    async fn list(&self, #[from_auth(self.validate)] _u: FakeUser)
        -> impl futures::stream::Stream<Item = String> + Send + 'static
    { async_stream::stream! { yield "l".into(); } }

    #[plexus_macros::method]
    async fn get(&self, #[from_auth(self.validate)] _u: FakeUser)
        -> impl futures::stream::Stream<Item = String> + Send + 'static
    { async_stream::stream! { yield "g".into(); } }
}

// ─── auth = "required" + explicit #[method(public)] escape hatch ─

struct StrictWithPublicEscape;

impl StrictWithPublicEscape {
    #[allow(dead_code)]
    async fn validate(&self, _: &AuthContext) -> Result<FakeUser, PlexusError> { Ok(FakeUser) }
}

#[plexus_macros::activation(
    namespace = "strict_with_escape",
    request = Req,
    auth = "required",
    crate_path = "plexus_core",
)]
impl StrictWithPublicEscape {
    #[plexus_macros::method]
    async fn authed(&self, #[from_auth(self.validate)] _u: FakeUser)
        -> impl futures::stream::Stream<Item = String> + Send + 'static
    { async_stream::stream! { yield "a".into(); } }

    #[plexus_macros::method(public)]
    async fn public_health(&self) -> impl futures::stream::Stream<Item = String> + Send + 'static
    { async_stream::stream! { yield "h".into(); } }
}

// ─── auth = "none", no method uses #[from_auth] ──────────────────

struct Public;

#[plexus_macros::activation(
    namespace = "public",
    auth = "none",
    crate_path = "plexus_core",
)]
impl Public {
    #[plexus_macros::method]
    async fn ping(&self) -> impl futures::stream::Stream<Item = String> + Send + 'static
    { async_stream::stream! { yield "p".into(); } }

    #[plexus_macros::method]
    async fn pong(&self) -> impl futures::stream::Stream<Item = String> + Send + 'static
    { async_stream::stream! { yield "q".into(); } }
}

// ─── auth = "optional" (the default), no explicit attribute ──────

struct DefaultOptional;

impl DefaultOptional {
    #[allow(dead_code)]
    async fn validate(&self, _: &AuthContext) -> Result<FakeUser, PlexusError> { Ok(FakeUser) }
}

// Uniformly-authed: no warning under "optional" default
#[plexus_macros::activation(
    namespace = "default_optional",
    request = Req,
    crate_path = "plexus_core",
)]
impl DefaultOptional {
    #[plexus_macros::method]
    async fn list(&self, #[from_auth(self.validate)] _u: FakeUser)
        -> impl futures::stream::Stream<Item = String> + Send + 'static
    { async_stream::stream! { yield "l".into(); } }
}

// ─── Tests ──────────────────────────────────────────────────────

#[test]
fn strict_activation_compiles_when_all_methods_authed() {
    let h = Strict;
    let _ = h.plugin_schema();
}

#[test]
fn strict_activation_allows_public_escape_hatch() {
    let h = StrictWithPublicEscape;
    let _ = h.plugin_schema();
}

#[test]
fn public_activation_compiles_when_no_method_uses_from_auth() {
    let h = Public;
    let _ = h.plugin_schema();
}

#[test]
fn default_optional_compiles_as_today() {
    let h = DefaultOptional;
    let _ = h.plugin_schema();
}

#[test]
fn compile_fail_cases() {
    let t = trybuild::TestCases::new();
    // auth = "required" + method without from_auth/public → compile error
    t.compile_fail("tests/compile/red6_required_rejects_unauth_method.rs");
    // auth = "none" + method with from_auth → compile error
    t.compile_fail("tests/compile/red6_none_rejects_from_auth.rs");
    // auth = "invalid-value" → parse error
    t.compile_fail("tests/compile/red6_invalid_posture_value.rs");
}
