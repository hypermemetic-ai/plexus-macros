//! REQ-4 acceptance tests: plugin_schema() includes 'request' JSON Schema blob.
//!
//! Run with: cargo test --test activation_schema_tests

use plexus_core::plexus::Activation;
use plexus_macros::PlexusRequest;
use std::net::SocketAddr;

// PlexusRequest already generates impl schemars::JsonSchema — no need for the separate derive.
#[derive(PlexusRequest)]
struct TestRequest {
    /// JWT from Keycloak
    #[from_cookie("access_token")]
    auth_token: String,

    #[from_peer]
    peer_addr: Option<SocketAddr>,
}

struct TestHub;

#[plexus_macros::activation(
    namespace = "test",
    version = "1.0.0",
    description = "Test hub",
    request = TestRequest,
    crate_path = "plexus_core"
)]
impl TestHub {
    #[plexus_macros::method(description = "list")]
    async fn list(
        &self,
        #[activation_param] auth_token: String,
        search: Option<String>,
    ) -> impl futures::stream::Stream<Item = String> + Send + 'static {
        async_stream::stream! {
            let _ = search;
            yield auth_token;
        }
    }
}

#[test]
fn plugin_schema_includes_request_field() {
    let hub = TestHub;
    let schema = hub.plugin_schema();
    let request = schema.request.expect("plugin_schema must include 'request' field");
    let props = &request["properties"];
    assert!(
        props["auth_token"].is_object(),
        "request schema must have auth_token property"
    );
    let source = &props["auth_token"]["x-plexus-source"];
    assert_eq!(source["from"], "cookie");
    assert_eq!(source["key"], "access_token");
}

#[test]
fn plugin_schema_request_required_matches_non_option_fields() {
    let hub = TestHub;
    let schema = hub.plugin_schema();
    let request = schema.request.expect("request schema must be present");
    let required = request["required"]
        .as_array()
        .expect("required array must be present");
    let names: Vec<&str> = required.iter().map(|v| v.as_str().unwrap()).collect();
    assert!(
        names.contains(&"auth_token"),
        "auth_token (String) must be in required"
    );
    assert!(
        !names.contains(&"peer_addr"),
        "peer_addr (Option) must NOT be in required"
    );
}

#[test]
fn plugin_schema_request_has_derived_source_on_peer() {
    let hub = TestHub;
    let schema = hub.plugin_schema();
    let request = schema.request.expect("request schema must be present");
    let source = &request["properties"]["peer_addr"]["x-plexus-source"];
    assert_eq!(source["from"], "derived");
}

#[test]
fn activation_with_no_request_has_no_request_field() {
    struct NoReqHub;

    #[plexus_macros::activation(namespace = "noreq", version = "1.0.0", crate_path = "plexus_core")]
    impl NoReqHub {
        #[plexus_macros::method(description = "ping")]
        async fn ping(&self) -> impl futures::stream::Stream<Item = String> + Send + 'static {
            async_stream::stream! { yield "pong".into(); }
        }
    }

    let hub = NoReqHub;
    let schema = hub.plugin_schema();
    assert!(
        schema.request.is_none(),
        "activation without request = Type must have no request field in schema"
    );
}

#[test]
fn activation_param_appears_in_method_schema_with_source_annotation() {
    // REQ-6: activation-level request fields are merged into every method's
    // params schema (unless overridden by the method with `request = ()`).
    // The injected fields carry `x-plexus-source` identifying where they come
    // from (cookie / header / query / derived), so clients reading the method
    // schema can see the full contract.
    let hub = TestHub;
    let schema = hub.plugin_schema();
    let schema_json = serde_json::to_value(&schema).unwrap();

    let methods = schema_json["methods"].as_array().unwrap();
    let list = methods.iter().find(|m| m["name"].as_str() == Some("list")).unwrap();

    // search IS an RPC param and MUST appear in the params schema.
    let params = &list["params"];
    let props = params["properties"]
        .as_object()
        .expect("params.properties must be an object");
    assert!(props.contains_key("search"), "search must appear in method params");

    // auth_token comes from the activation-level request struct; REQ-6 surfaces
    // it in the method's params with an x-plexus-source annotation.
    assert!(
        props.contains_key("auth_token"),
        "REQ-6: activation request field auth_token must be merged into method params"
    );
    assert_eq!(
        props["auth_token"]["x-plexus-source"]["from"],
        "cookie",
        "auth_token x-plexus-source.from should be 'cookie' (inherited from request struct)"
    );
    assert_eq!(
        props["auth_token"]["x-plexus-source"]["key"],
        "access_token",
        "auth_token x-plexus-source.key should be 'access_token'"
    );

    // peer_addr also inherits via the activation's request struct.
    assert!(props.contains_key("peer_addr"));
    assert_eq!(
        props["peer_addr"]["x-plexus-source"]["from"],
        "derived",
        "peer_addr x-plexus-source.from should be 'derived' (#[from_peer])"
    );
}
