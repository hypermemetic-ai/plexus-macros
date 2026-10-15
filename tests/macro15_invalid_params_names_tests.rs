//! MACRO-15: a parameter that does not deserialize is `InvalidParams`
//! naming the parameter — `id: <the type's message>` — on the one-param
//! path, the many-param path, the `Option` path and the bare-value
//! shortcut. The error variant (and so its code, -32602) is unchanged, and
//! the type's own message stays the tail.

use futures::stream::Stream;
use plexus_core::plexus::{Activation, PlexusError};
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

/// A ticket id, `SHORT-N` — refused by `TryFrom` otherwise.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(transparent)]
struct TicketId(String);

impl TryFrom<String> for TicketId {
    type Error = String;
    fn try_from(s: String) -> Result<Self, String> {
        match s.split_once('-') {
            Some((a, n)) if !a.is_empty() && n.parse::<u32>().is_ok() => Ok(Self(s.to_uppercase())),
            _ => Err(format!("`{s}` is not a ticket id (SHORT-N)")),
        }
    }
}

impl<'de> Deserialize<'de> for TicketId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        TicketId::try_from(String::deserialize(d)?).map_err(serde::de::Error::custom)
    }
}

/// A lenient enum: canonical spellings plus an alias.
#[derive(Debug, Clone, Serialize, JsonSchema)]
enum Status {
    Ready,
    Coding,
}

impl<'de> Deserialize<'de> for Status {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        match s.as_str() {
            "Ready" | "ready" => Ok(Status::Ready),
            "Coding" | "active" => Ok(Status::Coding),
            _ => Err(serde::de::Error::custom(format!("unknown status `{s}`: expected one of Ready, Coding"))),
        }
    }
}

struct Hub;

#[plexus_macros::activation(namespace = "hub", version = "1.0.0", crate_path = "plexus_core")]
impl Hub {
    /// One param.
    #[plexus_macros::method]
    async fn get(&self, id: TicketId) -> impl Stream<Item = String> + Send + 'static {
        async_stream::stream! { yield id.0; }
    }

    /// One optional param.
    #[plexus_macros::method]
    async fn maybe(&self, keep: Option<TicketId>) -> impl Stream<Item = String> + Send + 'static {
        async_stream::stream! { yield format!("{keep:?}"); }
    }

    /// Two params of one type, and an enum.
    #[plexus_macros::method]
    async fn set_status(
        &self,
        id: TicketId,
        parent: Option<TicketId>,
        status: Status,
    ) -> impl Stream<Item = String> + Send + 'static {
        async_stream::stream! { yield format!("{id:?} {parent:?} {status:?}"); }
    }
}

async fn invalid(method: &str, params: serde_json::Value) -> String {
    match Hub.call(method, params.clone(), None, None).await {
        Err(PlexusError::InvalidParams(m)) => m,
        Err(other) => panic!("{method} {params}: expected InvalidParams, got {other:?}"),
        Ok(_) => panic!("{method} {params}: expected InvalidParams, got a stream"),
    }
}

#[tokio::test]
async fn a_newtype_refusal_names_the_parameter() {
    let m = invalid("get", serde_json::json!({ "id": "not_an_id" })).await;
    assert_eq!(m, "id: `not_an_id` is not a ticket id (SHORT-N)");
}

#[tokio::test]
async fn the_bare_value_shortcut_names_the_parameter() {
    let m = invalid("get", serde_json::json!("not_an_id")).await;
    assert_eq!(m, "id: `not_an_id` is not a ticket id (SHORT-N)");
}

#[tokio::test]
async fn the_option_path_names_the_parameter() {
    let m = invalid("maybe", serde_json::json!({ "keep": "x" })).await;
    assert_eq!(m, "keep: `x` is not a ticket id (SHORT-N)");
}

#[tokio::test]
async fn two_params_of_one_type_say_which() {
    let m = invalid("set_status", serde_json::json!({ "id": "MNG-1", "parent": "nope", "status": "Ready" })).await;
    assert_eq!(m, "parent: `nope` is not a ticket id (SHORT-N)");
    let m = invalid("set_status", serde_json::json!({ "id": "nope", "status": "Ready" })).await;
    assert_eq!(m, "id: `nope` is not a ticket id (SHORT-N)");
}

#[tokio::test]
async fn a_lenient_enum_refusal_names_the_parameter() {
    let m = invalid("set_status", serde_json::json!({ "id": "MNG-1", "status": "wobbly" })).await;
    assert_eq!(m, "status: unknown status `wobbly`: expected one of Ready, Coding");
}

#[tokio::test]
async fn serde_type_errors_are_named_too() {
    let m = invalid("set_status", serde_json::json!({ "id": 7, "status": "Ready" })).await;
    assert!(m.starts_with("id: invalid type: integer `7`"), "{m}");
}

#[tokio::test]
async fn a_missing_field_is_unchanged() {
    let m = invalid("set_status", serde_json::json!({ "id": "MNG-1" })).await;
    assert_eq!(m, "missing field: status");
    let m = invalid("get", serde_json::json!({})).await;
    assert_eq!(m, "missing field: id");
}

#[tokio::test]
async fn the_wire_code_is_unchanged() {
    let e = PlexusError::InvalidParams(invalid("get", serde_json::json!({ "id": "x" })).await);
    let item = plexus_core::plexus::error_item("hub", &e);
    assert_eq!(item["code"], "-32602");
    assert_eq!(item["error_kind"], "invalid_params");
    assert_eq!(item["message"], "Invalid params: id: `x` is not a ticket id (SHORT-N)");
}

#[tokio::test]
async fn good_params_still_pass() {
    assert!(Hub.call("get", serde_json::json!({ "id": "mng-1" }), None, None).await.is_ok());
    assert!(Hub.call("maybe", serde_json::json!({}), None, None).await.is_ok());
    assert!(Hub
        .call("set_status", serde_json::json!({ "id": "MNG-1", "status": "active" }), None, None)
        .await
        .is_ok());
}
