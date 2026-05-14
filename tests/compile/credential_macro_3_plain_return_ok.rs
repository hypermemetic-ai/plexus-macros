//! AUTHZ-CRED-MACRO-3 acceptance criterion #2:
//!
//! A method whose return type does NOT derive `Credentials` produces a
//! `MethodSchema` with `credentials: []` (the pre-ticket default). This
//! exercises the autoref-blanket fallback path: the trait's default
//! `marker_slice()` returns an empty slice when no inherent override exists
//! on `CredentialsRegistryProbe<RT>`.

use async_stream::stream;
use futures::stream::Stream;

/// Plain return type — does NOT derive `Credentials`. The macro probes this
/// type at activation expansion time; the autoref-blanket trait default
/// fires and returns an empty marker slice.
#[derive(schemars::JsonSchema, serde::Serialize, serde::Deserialize)]
pub struct EchoEvent {
    pub message: String,
}

struct EchoHub;

#[plexus_macros::activation(namespace = "echo", version = "1.0.0", crate_path = "plexus_core")]
impl EchoHub {
    /// Echoes a message back.
    #[plexus_macros::method(description = "echo")]
    async fn echo(&self, msg: String) -> impl Stream<Item = EchoEvent> + Send + 'static {
        stream! {
            yield EchoEvent { message: msg };
        }
    }
}

fn main() {
    use plexus_core::plexus::MethodEnumSchema;
    use plexus_core::plexus::MethodSchema;

    let schemas: Vec<MethodSchema> = EchoHubMethod::method_schemas();
    let echo = schemas
        .iter()
        .find(|m| m.name == "echo")
        .expect("echo schema present");

    // AC #2: credentials list is empty for non-deriving return types.
    assert_eq!(
        echo.credentials.len(),
        0,
        "non-deriving return type → empty credentials, got {:?}",
        echo.credentials,
    );
    assert!(echo.requires_credential.is_none(), "no implicit requires_credential");
}
