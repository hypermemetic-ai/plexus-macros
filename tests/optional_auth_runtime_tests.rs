//! AUTHZ-MACRO-OPTIONAL-AUTH-1 runtime acceptance tests.
//!
//! These tests exercise the generated `Activation::call` wrapper for handlers
//! that declare `auth: Option<&AuthContext>`. The contract:
//!
//! 1. Calling the optional-auth handler with `auth = None` succeeds and the
//!    handler observes `None`. (Pre-ticket behavior: this would 401.)
//! 2. Calling the optional-auth handler with `auth = Some(&ctx)` succeeds
//!    and the handler observes `Some(&ctx)`.
//! 3. Calling a `&AuthContext` handler with `auth = None` still produces
//!    `PlexusError::Unauthenticated` — i.e. we have NOT regressed the
//!    force-auth wrapper.
//!
//! The handler emits a distinctive string for each branch so the assertions
//! can read the wrapped stream and verify which branch ran.
//!
//! # PLX-91 — why these four calls go through `call_arc`
//!
//! This is the residual PLX-97 handed the switchover, acted on here. A
//! turn-native `#[method]` runs its handler inside a `'static` stream that
//! outlives `Activation::call`'s borrow, so the handler needs an OWNED handle
//! on the activation. `call(&self, ..)` cannot supply one — that is
//! `E0521: borrowed data escapes outside of method`, the exact error PLX-91's
//! first attempt stopped on. PLX-97's answer is
//! `Activation::call_arc(self: Arc<Self>, ..)`, which `DynamicHub` and the
//! generated `ChildRouter` both route through. A direct caller does the same:
//! hold the activation in an `Arc` and call `call_arc`. The auth contract
//! being asserted is unchanged — auth injection simply moved from a generated
//! per-arm template into `runtime::entry`'s `AuthRequirementIr` check.

use async_stream::stream;
use futures::stream::Stream;
use futures::StreamExt;
use plexus_core::plexus::{Activation, AuthContext, PlexusError};
use std::sync::Arc;

struct MixedRouter;

#[plexus_macros::activation(
    namespace = "mixed_auth_runtime",
    version = "1.0.0",
    auth = "mixed",
    crate_path = "plexus_core",
)]
impl MixedRouter {
    /// Optional-auth handler. Branches on `auth.is_some()` so the test can
    /// detect via the emitted string whether the wrapper passed `Some` or
    /// `None`.
    #[plexus_macros::method(public)]
    async fn maybe_authed(
        &self,
        auth: Option<&AuthContext>,
    ) -> impl Stream<Item = String> + Send + 'static {
        let label = match auth {
            Some(ctx) => format!("some:{}", ctx.user_id),
            None => "none".to_string(),
        };
        stream! { yield label; }
    }

    /// Force-auth handler. Wrapper should still 401 when no AuthContext is
    /// attached.
    #[plexus_macros::method]
    async fn force_authed(
        &self,
        auth: &AuthContext,
    ) -> impl Stream<Item = String> + Send + 'static {
        let id = auth.user_id.clone();
        stream! { yield id; }
    }
}

/// Drain the first `Data` event from a `PlexusStream` and extract the
/// payload content as a string. The macro wraps user streams in
/// `PlexusStreamItem` envelopes; for our `String`-yielding handlers, the
/// first data item is `Data { content: serde_json::Value::String(s), .. }`.
async fn first_string(
    mut stream: plexus_core::plexus::PlexusStream,
) -> Option<String> {
    while let Some(item) = stream.next().await {
        if let plexus_core::plexus::PlexusStreamItem::Data { content, .. } = item {
            if let serde_json::Value::String(s) = content {
                return Some(s);
            }
        }
    }
    None
}

fn make_auth_ctx() -> AuthContext {
    AuthContext::new(
        "alice".into(),
        "sess-1".into(),
        vec![],
        serde_json::Value::Null,
    )
}

#[tokio::test]
async fn optional_auth_receives_none_when_no_context() {
    let router = Arc::new(MixedRouter);
    let stream = Activation::call_arc(
            router,
            "maybe_authed",
            serde_json::json!({}),
            None,
            None,
        )
        .await
        .expect("optional-auth handler must accept None auth without erroring");
    let payload = first_string(stream).await.expect("handler emitted a string");
    assert_eq!(
        payload, "none",
        "AUTHZ-MACRO-OPTIONAL-AUTH-1: handler should observe None when no \
         AuthContext is attached; observed {payload:?}",
    );
}

#[tokio::test]
async fn optional_auth_receives_some_when_context_attached() {
    let router = Arc::new(MixedRouter);
    let ctx = make_auth_ctx();
    let stream = Activation::call_arc(
            router,
            "maybe_authed",
            serde_json::json!({}),
            Some(&ctx),
            None,
        )
        .await
        .expect("optional-auth handler must accept Some(&ctx)");
    let payload = first_string(stream).await.expect("handler emitted a string");
    assert_eq!(
        payload, "some:alice",
        "AUTHZ-MACRO-OPTIONAL-AUTH-1: handler should observe Some(&ctx) when \
         AuthContext is attached; observed {payload:?}",
    );
}

#[tokio::test]
async fn force_auth_still_rejects_missing_context() {
    let router = Arc::new(MixedRouter);
    // PlexusStream isn't `Debug` (it's a boxed trait object), so we can't
    // use `.expect_err(...)`; pattern-match on the Result instead.
    match Activation::call_arc(router, "force_authed", serde_json::json!({}), None, None)
        .await
    {
        Err(PlexusError::Unauthenticated(_)) => {}
        Err(other) => panic!(
            "expected PlexusError::Unauthenticated for missing auth on a \
             force-auth handler; got {other:?}"
        ),
        Ok(_) => panic!(
            "force-auth handler must still error when no AuthContext is \
             attached; got Ok"
        ),
    }
}

#[tokio::test]
async fn force_auth_accepts_attached_context() {
    let router = Arc::new(MixedRouter);
    let ctx = make_auth_ctx();
    let stream = Activation::call_arc(
            router,
            "force_authed",
            serde_json::json!({}),
            Some(&ctx),
            None,
        )
        .await
        .expect("force-auth handler must accept Some(&ctx)");
    let payload = first_string(stream).await.expect("handler emitted a string");
    assert_eq!(payload, "alice");
}
