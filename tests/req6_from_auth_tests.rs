//! REQ-6 acceptance tests: per-method auth resolver annotations and
//! per-method request override.

use plexus_core::plexus::{Activation, AuthContext, PlexusError};
use plexus_macros::PlexusRequest;

// --- Fixtures ----------------------------------------------------------

#[derive(Clone, Debug)]
struct FakeUser;

#[derive(PlexusRequest)]
struct MinimalReq {
    #[from_cookie("access_token")]
    auth_token: String,
}

struct AuthHub;

impl AuthHub {
    #[allow(dead_code)]
    async fn validate_user(&self, _ctx: &AuthContext) -> Result<FakeUser, PlexusError> { Ok(FakeUser) }
}

#[plexus_macros::activation(
    namespace = "auth_hub",
    version = "1.0.0",
    description = "REQ-6 auth hub",
    request = MinimalReq,
    crate_path = "plexus_core",
)]
impl AuthHub {
    /// List things
    #[plexus_macros::method(description = "list")]
    async fn list(
        &self,
        #[from_auth(self.validate_user)] _user: FakeUser,
        search: Option<String>,
    ) -> impl futures::stream::Stream<Item = String> + Send + 'static {
        async_stream::stream! {
            let _ = search;
            yield "ok".to_string();
        }
    }

    /// Skip-extraction method: request = () bypasses the activation-level merge.
    #[plexus_macros::method(description = "skip", request = ())]
    async fn skip(&self) -> impl futures::stream::Stream<Item = String> + Send + 'static {
        async_stream::stream! { yield "skipped".to_string(); }
    }
}

// --- Tests -------------------------------------------------------------

#[test]
fn from_auth_param_has_source_annotation() {
    let hub = AuthHub;
    let schema = hub.plugin_schema();
    let schema_json = serde_json::to_value(&schema).unwrap();

    let methods = schema_json["methods"].as_array().unwrap();
    let list = methods
        .iter()
        .find(|m| m["name"].as_str() == Some("list"))
        .expect("list method must appear");

    let props = list["params"]["properties"]
        .as_object()
        .expect("list.params.properties must be an object");

    // Auth param appears with x-plexus-source: { from: "auth", resolver: "..." }
    assert!(
        props.contains_key("_user"),
        "REQ-6: #[from_auth] param _user must appear in method params schema"
    );
    let source = &props["_user"]["x-plexus-source"];
    assert_eq!(source["from"], "auth", "#[from_auth] must tag source.from = auth");
    let resolver = source["resolver"].as_str().expect("resolver should be a string");
    assert!(
        resolver.contains("validate_user"),
        "resolver expression should contain 'validate_user'; got: {}",
        resolver
    );

    // Plain RPC param `search` is still present.
    assert!(props.contains_key("search"), "search must appear as RPC param");
}

#[test]
fn method_with_request_skip_omits_activation_request_fields() {
    let hub = AuthHub;
    let schema = hub.plugin_schema();
    let schema_json = serde_json::to_value(&schema).unwrap();

    let methods = schema_json["methods"].as_array().unwrap();
    let skip = methods
        .iter()
        .find(|m| m["name"].as_str() == Some("skip"))
        .expect("skip method must appear");

    // auth_token from the activation's MinimalReq must NOT appear on the skip method
    let props = skip["params"]["properties"].as_object();
    if let Some(p) = props {
        assert!(
            !p.contains_key("auth_token"),
            "REQ-6: method with `request = ()` override must not inherit activation-level request fields"
        );
    }
    // (The schema may legitimately have no properties at all for zero-param methods.)
}

#[test]
fn non_override_method_inherits_activation_request_fields() {
    let hub = AuthHub;
    let schema = hub.plugin_schema();
    let schema_json = serde_json::to_value(&schema).unwrap();

    let methods = schema_json["methods"].as_array().unwrap();
    let list = methods
        .iter()
        .find(|m| m["name"].as_str() == Some("list"))
        .expect("list method must appear");

    let props = list["params"]["properties"]
        .as_object()
        .expect("list.params.properties must be an object");

    // auth_token IS inherited from the activation's MinimalReq
    assert!(
        props.contains_key("auth_token"),
        "REQ-6: activation-level request fields must merge into non-override methods"
    );
    assert_eq!(
        props["auth_token"]["x-plexus-source"]["from"],
        "cookie",
        "inherited auth_token should carry its x-plexus-source from the request struct"
    );
}
