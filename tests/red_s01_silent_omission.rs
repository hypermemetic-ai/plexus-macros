//! RED-S01 spike: does a method that omits #[from_auth] in a mixed-auth activation
//! compile silently and remain unauthenticated, or does the macro emit a diagnostic?

use plexus_core::plexus::{Activation, AuthContext, PlexusError};
use plexus_macros::PlexusRequest;

// --- Fixtures ----------------------------------------------------------

#[derive(Clone, Debug)]
struct FakeUser;

#[derive(PlexusRequest)]
struct AuthReq {
    #[from_cookie("access_token")]
    auth_token: String,
}

struct AuthedHub;

impl AuthedHub {
    #[allow(dead_code)]
    async fn validate(&self, _ctx: &AuthContext) -> Result<FakeUser, PlexusError> {
        Ok(FakeUser)
    }
}

#[plexus_macros::activation(
    namespace = "authed_hub",
    version = "1.0.0",
    description = "RED-S01 mixed-auth hub",
    request = AuthReq,
    crate_path = "plexus_core",
)]
impl AuthedHub {
    /// Protected method with from_auth and an RPC param
    #[plexus_macros::method(description = "list authed")]
    async fn list(
        &self,
        #[from_auth(self.validate)] _user: FakeUser,
        search: Option<String>,
    ) -> impl futures::stream::Stream<Item = String> + Send + 'static {
        async_stream::stream! {
            let _ = search;
            yield "authed_list".to_string();
        }
    }

    /// Unprotected method — NO #[from_auth], but has RPC param
    #[plexus_macros::method(description = "leak unauthed")]
    async fn leak(&self, query: Option<String>) -> impl futures::stream::Stream<Item = String> + Send + 'static {
        async_stream::stream! {
            let _ = query;
            yield "unauthed_leak".to_string();
        }
    }
}

// --- Tests -------------------------------------------------------------

#[test]
fn red_s01_leak_method_compiles_and_lacks_auth_annotation() {
    let hub = AuthedHub;
    let schema = hub.plugin_schema();
    let schema_json = serde_json::to_value(&schema).unwrap();

    eprintln!("\n=== FULL SCHEMA ===\n{}", serde_json::to_string_pretty(&schema_json).unwrap());

    let methods = schema_json["methods"].as_array().unwrap();
    
    // Both methods must be present in the schema
    let list_method = methods
        .iter()
        .find(|m| m["name"].as_str() == Some("list"))
        .expect("list method must be in schema");
    
    let leak_method = methods
        .iter()
        .find(|m| m["name"].as_str() == Some("leak"))
        .expect("leak method must be in schema");

    eprintln!("\n=== LIST METHOD ===\n{}", serde_json::to_string_pretty(&list_method).unwrap());
    eprintln!("\n=== LEAK METHOD ===\n{}", serde_json::to_string_pretty(&leak_method).unwrap());

    // The 'list' method should have _user param with x-plexus-source.from = "auth"
    let empty_map_list = serde_json::Map::new();
    let list_params = list_method["params"]["properties"]
        .as_object()
        .unwrap_or(&empty_map_list);
    
    eprintln!("\n=== LIST PARAMS ===\n{}", serde_json::to_string_pretty(list_params).unwrap());

    assert!(
        list_params.contains_key("_user"),
        "list must have _user param"
    );
    let list_source = &list_params["_user"]["x-plexus-source"];
    assert_eq!(
        list_source["from"], "auth",
        "list._user must have from='auth'"
    );

    // The 'leak' method should have NO params with from="auth"
    // It will inherit auth_token from activation-level request, but that should be from="cookie"
    let empty_map_leak = serde_json::Map::new();
    let leak_params = leak_method["params"]["properties"]
        .as_object()
        .unwrap_or(&empty_map_leak);
    
    eprintln!("\n=== LEAK PARAMS ===\n{}", serde_json::to_string_pretty(leak_params).unwrap());

    // Check that no param in leak has from="auth"
    for (param_name, param_schema) in leak_params.iter() {
        let source = param_schema.get("x-plexus-source");
        if let Some(s) = source {
            if let Some(from) = s.get("from") {
                assert_ne!(
                    from, "auth",
                    "leak method param '{}' should NOT have from='auth', but does",
                    param_name
                );
            }
        }
    }

    println!(
        "\n*** RED-S01 HOLE CONFIRMED ***\n\
         leak method compiled silently.\n\
         leak method schema has NO from='auth' params.\n\
         unauthenticated method can ship undetected."
    );
}
