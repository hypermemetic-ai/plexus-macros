//! AUTHZ-MACRO-OPTIONAL-AUTH-1 compile-pass fixture.
//!
//! Goal: prove that a handler declared as
//!   `async fn foo(&self, auth: Option<&AuthContext>, ...) -> ...`
//! compiles cleanly under `#[plexus_macros::activation]` and that the
//! generated dispatch wrapper hands the handler an `Option<&AuthContext>`
//! (instead of force-unwrapping the request extension into `&AuthContext`).
//!
//! The fixture intentionally mixes a force-auth handler (`write`,
//! `auth: &AuthContext`) and an optional-auth handler (`read`,
//! `auth: Option<&AuthContext>`) in the same activation under
//! `auth = "mixed"`, which is the trak-style "anonymous read, authed write"
//! shape this ticket enables.
//!
//! A separate single-handler activation (`AnonOnly`) exercises the
//! optional shape on its own to keep the type-check surface minimal.
//!
//! See `tests/optional_auth_runtime_tests.rs` for the runtime assertion
//! that the handler receives `None` when no `AuthContext` is attached.

use async_stream::stream;
use futures::stream::Stream;
use plexus_core::plexus::AuthContext;

struct Mixed;

#[plexus_macros::activation(
    namespace = "mixed_auth_demo",
    version = "1.0.0",
    auth = "mixed",
    crate_path = "plexus_core",
)]
impl Mixed {
    /// Anonymous-friendly read. Wrapper must pass `Option<&AuthContext>`
    /// straight through without erroring on missing context.
    #[plexus_macros::method(public)]
    async fn read(
        &self,
        auth: Option<&AuthContext>,
    ) -> impl Stream<Item = String> + Send + 'static {
        let label = match auth {
            Some(_) => "authed-read",
            None => "anon-read",
        };
        stream! { yield label.into(); }
    }

    /// Auth-required write. Wrapper must keep today's force-unwrap-or-error
    /// behavior here — caller without AuthContext gets Unauthenticated.
    #[plexus_macros::method]
    async fn write(
        &self,
        auth: &AuthContext,
        payload: String,
    ) -> impl Stream<Item = String> + Send + 'static {
        // Touch `auth` so the compiler doesn't lint it as unused — the macro
        // strips the parameter from the RPC schema either way.
        let _ = auth.user_id.as_str();
        stream! { yield payload; }
    }
}

struct AnonOnly;

#[plexus_macros::activation(
    namespace = "anon_only_demo",
    version = "1.0.0",
    auth = "none",
    crate_path = "plexus_core",
)]
impl AnonOnly {
    /// Single anonymous-friendly handler, no auth required at all.
    #[plexus_macros::method(public)]
    async fn ping(
        &self,
        auth: Option<&AuthContext>,
    ) -> impl Stream<Item = String> + Send + 'static {
        let label = if auth.is_some() { "authed" } else { "anon" };
        stream! { yield label.into(); }
    }
}

fn main() {}
