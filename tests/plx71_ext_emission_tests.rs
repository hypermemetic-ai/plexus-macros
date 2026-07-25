//! PLX-71 acceptance test: the `#[method(ext(...))]` open-extension passthrough
//! flows string / integer / bool literals all the way through codegen into the
//! generated `MethodSchema.extensions` map, and methods without `ext(...)` keep
//! an empty map (byte-identical wire).
//!
//! Verifies acceptance criterion #3 (parser passthrough handles string/int/bool
//! and populates the schema end-to-end through codegen) and reinforces #4's
//! positive half (a method with no `ext(...)` serializes with NO `extensions`
//! key).
//!
//! Run with: `cargo test --test plx71_ext_emission_tests`.

use async_stream::stream;
use futures::stream::Stream;

#[derive(Clone)]
struct ExtFixture;

#[plexus_macros::activation(
    namespace = "ext_fixture",
    version = "1.0.0",
    crate_path = "plexus_core"
)]
impl ExtFixture {
    /// Carries three extension pairs across all three literal types, plus a
    /// first-class typed attribute (`http_method`) that must stay first-class.
    #[plexus_macros::method(
        http_method = "POST",
        ext(nostr_kind = 30023, label = "x", flag = true)
    )]
    async fn publish_node(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "p".into(); }
    }

    /// No `ext(...)` — extensions must stay empty and be omitted from the wire.
    #[plexus_macros::method]
    async fn plain(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "q".into(); }
    }
}

fn find(schemas: &[plexus_core::plexus::MethodSchema], name: &str) -> plexus_core::plexus::MethodSchema {
    schemas
        .iter()
        .find(|m| m.name == name)
        .unwrap_or_else(|| panic!("method `{name}` missing from schemas"))
        .clone()
}

/// PLX-71 AC #3/#4: the three typed values land in `extensions`, the typed
/// `http_method` stays first-class, and the values survive a wire round-trip.
#[test]
fn ext_pairs_populate_schema_extensions() {
    let schemas = ExtFixtureMethod::method_schemas();
    let publish = find(&schemas, "publish_node");

    assert_eq!(publish.extensions.len(), 3, "all three ext pairs must be present");
    assert_eq!(
        publish.extensions.get("nostr_kind"),
        Some(&serde_json::json!(30023)),
        "integer literal → JSON number"
    );
    assert_eq!(
        publish.extensions.get("label"),
        Some(&serde_json::json!("x")),
        "string literal → JSON string"
    );
    assert_eq!(
        publish.extensions.get("flag"),
        Some(&serde_json::json!(true)),
        "bool literal → JSON bool"
    );

    // First-class typed attribute stays first-class (not swallowed by ext).
    assert_eq!(publish.http_method, plexus_core::plexus::schema::HttpMethod::Post);

    // Wire round-trip: extensions surface in JSON and decode back losslessly.
    let json = serde_json::to_value(&publish).expect("schema serializes");
    assert_eq!(json["extensions"]["nostr_kind"], serde_json::json!(30023));
    assert_eq!(json["extensions"]["label"], serde_json::json!("x"));
    assert_eq!(json["extensions"]["flag"], serde_json::json!(true));
    let decoded: plexus_core::plexus::MethodSchema =
        serde_json::from_value(json).expect("schema deserializes");
    assert_eq!(decoded.extensions, publish.extensions);
}

/// PLX-71 AC #4 (positive half): a method with no `ext(...)` has an empty
/// extensions map and NO `extensions` key on the wire.
#[test]
fn method_without_ext_omits_extensions_from_wire() {
    let schemas = ExtFixtureMethod::method_schemas();
    let plain = find(&schemas, "plain");

    assert!(plain.extensions.is_empty(), "no ext(...) → empty map");

    let json = serde_json::to_value(&plain).expect("schema serializes");
    assert!(
        !json.as_object().unwrap().contains_key("extensions"),
        "extensions key must be omitted when empty, got {json}"
    );
}
