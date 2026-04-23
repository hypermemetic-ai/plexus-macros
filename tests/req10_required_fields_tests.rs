//! REQ-10 acceptance tests for activation `required = [...]` field-locking.
//!
//! The compile-failure cases live in `tests/compile/req10_*.rs` and are
//! exercised by trybuild. Here we verify the compile-success paths: an
//! activation with `required = []` (or no required list) allows per-method
//! `request = ()` overrides just like today.

use plexus_core::plexus::Activation;
use plexus_macros::PlexusRequest;

#[derive(PlexusRequest)]
struct SomeRequest {
    #[from_cookie("access_token")]
    auth_token: String,
}

// --- Case A: activation with no `required` attribute --------------------

struct HubNoReq;

#[plexus_macros::activation(
    namespace = "hub_no_req",
    version = "1.0.0",
    description = "REQ-10 baseline",
    request = SomeRequest,
    crate_path = "plexus_core",
)]
impl HubNoReq {
    /// Ping with override — allowed because no `required` list on activation
    #[plexus_macros::method(description = "ping", request = ())]
    async fn ping(&self) -> impl futures::stream::Stream<Item = String> + Send + 'static {
        async_stream::stream! { yield "pong".into(); }
    }
}

#[test]
fn compile_fail_cases() {
    let t = trybuild::TestCases::new();
    // Activation with `required = [origin]` + method with `request = ()` must fail.
    t.compile_fail("tests/compile/req10_required_rejects_override.rs");
}

#[test]
fn override_allowed_when_activation_has_no_required_list() {
    // If we compiled, the macro accepted request = () without the lock list.
    let hub = HubNoReq;
    let schema = hub.plugin_schema();
    let schema_json = serde_json::to_value(&schema).unwrap();
    let methods = schema_json["methods"].as_array().unwrap();
    let ping = methods.iter().find(|m| m["name"].as_str() == Some("ping")).unwrap();
    // The ping method should have no merged auth_token (override skipped the merge)
    let props = ping["params"]["properties"].as_object();
    if let Some(p) = props {
        assert!(
            !p.contains_key("auth_token"),
            "override without required list must drop activation-level fields"
        );
    }
}
