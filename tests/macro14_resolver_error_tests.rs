//! MACRO-14: a `#[from_auth]` resolver's refusal reaches the wire typed.
//!
//! A resolver whose error is already a `PlexusError` (a `Forbidden`, an
//! `Unauthenticated`) is answered as that error — its code and
//! `error_kind`, and for a missing grant `grant` / `scope` / `roles` /
//! `held_on`, exactly as plexus-core's `error_item` writes them for the
//! gate. Any other `Display` error keeps today's mapping:
//! `ExecutionError(e.to_string())`, `-32000`.

use futures::stream::Stream;
use plexus_auth_core::grants::{Grant, ScopeRef};
use plexus_auth_core::RoleName;
use plexus_core::plexus::{error_item, Activation, AuthContext, AuthzDenyReason, PlexusError};
use plexus_macros::PlexusRequest;

#[derive(Clone, Debug)]
struct User(String);

#[derive(PlexusRequest)]
#[allow(dead_code)]
struct Req {
    #[from_cookie("access_token")]
    token: Option<String>,
}

/// A resolver error that is not a `PlexusError` — today's mapping holds.
#[derive(Debug)]
struct NotYou;

impl std::fmt::Display for NotYou {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("not you")
    }
}

struct Hub;

fn missing_grant() -> PlexusError {
    PlexusError::Forbidden {
        reason: AuthzDenyReason::MissingGrant {
            grant: Grant::new("hub.typed"),
            scope: ScopeRef::named("MENGER"),
            roles: vec![RoleName::new("editor"), RoleName::new("admin")],
            held_on: vec![ScopeRef::named("OTHER")],
        },
    }
}

impl Hub {
    async fn typed(&self, ctx: &AuthContext) -> Result<User, PlexusError> {
        match ctx.user_id.as_str() {
            "alice" => Ok(User("alice".into())),
            "anon" => Err(PlexusError::Unauthenticated("sign in".into())),
            _ => Err(missing_grant()),
        }
    }

    async fn stringly(&self, ctx: &AuthContext) -> Result<User, String> {
        if ctx.user_id == "alice" { Ok(User("alice".into())) } else { Err("no such user".into()) }
    }

    async fn displayed(&self, _ctx: &AuthContext) -> Result<User, NotYou> {
        Err(NotYou)
    }
}

#[plexus_macros::activation(namespace = "hub", version = "1.0.0", request = Req, crate_path = "plexus_core")]
impl Hub {
    /// A resolver returning `PlexusError`.
    #[plexus_macros::method]
    async fn typed_call(&self, #[from_auth(self.typed)] user: User) -> impl Stream<Item = String> + Send + 'static {
        async_stream::stream! { yield user.0; }
    }

    /// A resolver returning `String`.
    #[plexus_macros::method]
    async fn string_call(&self, #[from_auth(self.stringly)] user: User) -> impl Stream<Item = String> + Send + 'static {
        async_stream::stream! { yield user.0; }
    }

    /// A resolver returning another `Display` error.
    #[plexus_macros::method]
    async fn display_call(&self, #[from_auth(self.displayed)] user: User) -> impl Stream<Item = String> + Send + 'static {
        async_stream::stream! { yield user.0; }
    }
}

fn ctx(user: &str) -> AuthContext {
    AuthContext::new(user.into(), "sess".into(), vec![], serde_json::Value::Null)
}

async fn refusal(method: &str, user: &str) -> PlexusError {
    match Hub.call(method, serde_json::json!({}), Some(&ctx(user)), None).await {
        Err(e) => e,
        Ok(_) => panic!("{method} as {user}: expected a refusal"),
    }
}

#[tokio::test]
async fn a_plexus_forbidden_reaches_the_wire_typed() {
    let e = refusal("typed_call", "mallory").await;
    assert!(
        matches!(&e, PlexusError::Forbidden { reason: AuthzDenyReason::MissingGrant { .. } }),
        "MACRO-14: the resolver's Forbidden must pass through, got {e:?}"
    );
    let item = error_item("hub", &e);
    assert_eq!(item["type"], "error");
    assert_eq!(item["code"], "-32003");
    assert_eq!(item["error_kind"], "forbidden");
    assert_eq!(item["grant"], "hub.typed");
    assert_eq!(item["scope"], "MENGER");
    assert_eq!(item["roles"], serde_json::json!(["editor", "admin"]));
    assert_eq!(item["held_on"], serde_json::json!(["OTHER"]));
    assert_eq!(item["message"], "needs hub.typed on MENGER");
}

#[tokio::test]
async fn a_plexus_unauthenticated_reaches_the_wire_typed() {
    let e = refusal("typed_call", "anon").await;
    assert!(matches!(&e, PlexusError::Unauthenticated(m) if m == "sign in"), "got {e:?}");
    let item = error_item("hub", &e);
    assert_eq!(item["code"], "-32001");
    assert_eq!(item["error_kind"], "unauthenticated");
}

#[tokio::test]
async fn a_string_resolver_error_is_still_an_execution_error() {
    let e = refusal("string_call", "mallory").await;
    assert!(matches!(&e, PlexusError::ExecutionError(m) if m == "no such user"), "got {e:?}");
    let item = error_item("hub", &e);
    assert_eq!(item["code"], "-32000");
    assert!(item.get("error_kind").is_none(), "an execution error carries no error_kind");
}

#[tokio::test]
async fn another_display_error_is_still_an_execution_error() {
    let e = refusal("display_call", "alice").await;
    assert!(matches!(&e, PlexusError::ExecutionError(m) if m == "not you"), "got {e:?}");
}

#[tokio::test]
async fn an_admitted_caller_still_reaches_the_method() {
    for method in ["typed_call", "string_call"] {
        assert!(Hub.call(method, serde_json::json!({}), Some(&ctx("alice")), None).await.is_ok(), "{method}");
    }
}
