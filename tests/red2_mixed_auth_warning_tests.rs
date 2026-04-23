//! RED-2 acceptance tests: macro warns on mixed-auth activations
//! and respects the documented opt-outs.
//!
//! NOTE: rustc warnings can't be assertion-checked from a `#[test]` —
//! they only land in compiler diagnostic output. We verify the
//! warning's BEHAVIOR (suppressions don't break the build) and
//! observe the warning's PRESENCE manually via `cargo build` (or
//! a trybuild-based test in a follow-up).
//!
//! What's covered here:
//! - All four shape combinations compile cleanly
//! - The opt-outs (`auth_posture = "mixed"` and `#[method(public)]`)
//!   are accepted by the parser
//! - Activations that have NO mix and NO opt-outs work unchanged

use plexus_core::plexus::{Activation, AuthContext, PlexusError};
use plexus_macros::PlexusRequest;

#[derive(Clone, Debug)]
struct FakeUser;

#[derive(PlexusRequest)]
struct Req {
    #[from_cookie("access_token")]
    auth_token: String,
}

// ─── Shape A: uniformly auth'd — no warning expected ──────────────

struct UniformlyAuthed;

impl UniformlyAuthed {
    #[allow(dead_code)]
    async fn validate(&self, _: &AuthContext) -> Result<FakeUser, PlexusError> { Ok(FakeUser) }
}

#[plexus_macros::activation(
    namespace = "uniformly_authed",
    request = Req,
    crate_path = "plexus_core",
)]
impl UniformlyAuthed {
    #[plexus_macros::method]
    async fn list(&self, #[from_auth(self.validate)] _u: FakeUser)
        -> impl futures::stream::Stream<Item = String> + Send + 'static
    { async_stream::stream! { yield "a".into(); } }

    #[plexus_macros::method]
    async fn get(&self, #[from_auth(self.validate)] _u: FakeUser)
        -> impl futures::stream::Stream<Item = String> + Send + 'static
    { async_stream::stream! { yield "b".into(); } }
}

// ─── Shape B: uniformly public — no warning expected ──────────────

struct UniformlyPublic;

#[plexus_macros::activation(
    namespace = "uniformly_public",
    crate_path = "plexus_core",
)]
impl UniformlyPublic {
    #[plexus_macros::method]
    async fn ping(&self) -> impl futures::stream::Stream<Item = String> + Send + 'static
    { async_stream::stream! { yield "p".into(); } }

    #[plexus_macros::method]
    async fn pong(&self) -> impl futures::stream::Stream<Item = String> + Send + 'static
    { async_stream::stream! { yield "q".into(); } }
}

// ─── Shape C: mixed, opted in via auth_posture = "mixed" ──────────
// This proves the activation-level opt-out parses + suppresses.

struct MixedActivationOptOut;

impl MixedActivationOptOut {
    #[allow(dead_code)]
    async fn validate(&self, _: &AuthContext) -> Result<FakeUser, PlexusError> { Ok(FakeUser) }
}

#[plexus_macros::activation(
    namespace = "mixed_activation_opt_out",
    request = Req,
    auth_posture = "mixed",
    crate_path = "plexus_core",
)]
impl MixedActivationOptOut {
    #[plexus_macros::method]
    async fn authed(&self, #[from_auth(self.validate)] _u: FakeUser)
        -> impl futures::stream::Stream<Item = String> + Send + 'static
    { async_stream::stream! { yield "a".into(); } }

    #[plexus_macros::method]
    async fn unauthed(&self) -> impl futures::stream::Stream<Item = String> + Send + 'static
    { async_stream::stream! { yield "u".into(); } }
}

// ─── Shape D: mixed, opted in per-method via #[method(public)] ────
// Proves the per-method opt-out parses + suppresses.

struct MixedPerMethodOptOut;

impl MixedPerMethodOptOut {
    #[allow(dead_code)]
    async fn validate(&self, _: &AuthContext) -> Result<FakeUser, PlexusError> { Ok(FakeUser) }
}

#[plexus_macros::activation(
    namespace = "mixed_per_method_opt_out",
    request = Req,
    crate_path = "plexus_core",
)]
impl MixedPerMethodOptOut {
    #[plexus_macros::method]
    async fn authed(&self, #[from_auth(self.validate)] _u: FakeUser)
        -> impl futures::stream::Stream<Item = String> + Send + 'static
    { async_stream::stream! { yield "a".into(); } }

    #[plexus_macros::method(public)]
    async fn intentionally_public(&self)
        -> impl futures::stream::Stream<Item = String> + Send + 'static
    { async_stream::stream! { yield "p".into(); } }
}

// ─── Tests ────────────────────────────────────────────────────────

#[test]
fn uniformly_authed_compiles_cleanly() {
    let h = UniformlyAuthed;
    let _ = h.plugin_schema();
}

#[test]
fn uniformly_public_compiles_cleanly() {
    let h = UniformlyPublic;
    let _ = h.plugin_schema();
}

#[test]
fn mixed_with_activation_opt_out_compiles_cleanly() {
    let h = MixedActivationOptOut;
    let _ = h.plugin_schema();
}

#[test]
fn mixed_with_per_method_opt_out_compiles_cleanly() {
    let h = MixedPerMethodOptOut;
    let _ = h.plugin_schema();
}
