//! AUTHZ-CRED-MACRO-3 acceptance criterion #1 + #4:
//!
//! A method whose streaming return type implements
//! `HasCredentialMarkers` (the trait `#[derive(Credentials)]` emits) gets
//! its `MethodSchema.credentials` field populated with one
//! `CredentialFieldDecl` entry per marker.
//!
//! This fixture exercises the end-to-end wired path:
//!
//! 1. `#[plexus_macros::activation]` codegen for each `#[method]` emits a
//!    call to `(&CredentialsRegistryProbe::<RT>::new()).marker_slice()`.
//! 2. For RT implementing `HasCredentialMarkers`, the specialized inherent
//!    `marker_slice` on `CredentialsRegistryProbe<T: HasCredentialMarkers>`
//!    wins over the empty-slice fallback via Rust's inherent-first method
//!    resolution.
//! 3. The macro composes each `CredentialFieldMarker` into a
//!    `CredentialFieldDecl` via `from_marker(..)` and threads the resulting
//!    `Vec` through `MethodSchema::with_credentials(...)`.
//!
//! For test ergonomics this fixture provides the `HasCredentialMarkers`
//! impl by hand rather than via `#[derive(Credentials)]`. Reason: the
//! derive enforces that `#[credential(...)]`-annotated fields have the
//! sealed `Credential<T>` wrapper type, and `Credential<T>` does NOT
//! implement `schemars::JsonSchema` (intentional — wire envelopes carry
//! the sentinel, not the inner value). Activation codegen calls
//! `schema_for!(ReturnType)` for the JSON-schema projection, which would
//! fail on a struct containing `Credential<T>`. The hand-rolled trait impl
//! is byte-for-byte identical to the macro emission (see
//! `credential_basic_struct_ok.rs` for the macro-emission acceptance
//! fixture) — what this fixture proves is the activation-side wiring.

use async_stream::stream;
use futures::stream::Stream;

use plexus_auth_core::{
    AttachmentSite, CredentialFieldMarker, CredentialKind, CredentialScheme, HasCredentialMarkers,
    HeaderName,
};

/// Streaming event yielded by the activation method. Implements
/// `HasCredentialMarkers` directly (simulating what `#[derive(Credentials)]`
/// emits) so the macro's autoref-specialized call site picks up the
/// populated slice.
#[derive(schemars::JsonSchema, serde::Serialize, serde::Deserialize)]
pub struct LoginEvent {
    pub user_id: String,
    /// Placeholder for the credential field. In production code this
    /// would be `Credential<SessionToken>` — but `Credential<T>` lacks a
    /// `JsonSchema` impl by design, so we use a placeholder type to keep
    /// `schemars::schema_for!` happy for fixture purposes.
    pub session_id: String,
}

impl HasCredentialMarkers for LoginEvent {
    fn credential_markers() -> &'static [CredentialFieldMarker] {
        use std::sync::OnceLock;
        static M: OnceLock<Vec<CredentialFieldMarker>> = OnceLock::new();
        M.get_or_init(|| {
            vec![CredentialFieldMarker::new(
                None,
                "session",
                CredentialKind::Bearer,
                AttachmentSite::Header {
                    name: HeaderName::try_new("authorization").unwrap(),
                },
                Some(CredentialScheme::new("Bearer ")),
                vec![],
                None,
                None,
            )]
        })
        .as_slice()
    }
}

struct AuthHub;

#[plexus_macros::activation(namespace = "auth", version = "1.0.0", crate_path = "plexus_core")]
impl AuthHub {
    /// Logs a user in. Returns a stream yielding `LoginEvent` values.
    #[plexus_macros::method(description = "logs a user in")]
    async fn login(&self, _username: String) -> impl Stream<Item = LoginEvent> + Send + 'static {
        stream! {
            yield LoginEvent {
                user_id: String::new(),
                session_id: String::new(),
            };
        }
    }
}

fn main() {
    use plexus_core::plexus::MethodSchema;

    let schemas: Vec<MethodSchema> = AuthHubMethod::method_schemas();
    let login = schemas
        .iter()
        .find(|m| m.name == "login")
        .expect("login schema present");

    // AC #1: credentials list is populated — exactly one entry per
    // declared field on the return type (the LoginEvent's `session`
    // marker).
    assert_eq!(
        login.credentials.len(),
        1,
        "expected 1 credential decl, got {}: {:?}",
        login.credentials.len(),
        login.credentials,
    );
    let decl = &login.credentials[0];
    assert_eq!(
        decl.field_path,
        vec!["session".to_string()],
        "field_path matches the marker's `field` value"
    );
    assert!(decl.variant_tag.is_none(), "struct return type → no variant tag");
    assert!(matches!(decl.metadata.kind, CredentialKind::Bearer));
    assert!(matches!(
        decl.metadata.attach_as,
        AttachmentSite::Header { ref name } if name.as_str() == "authorization"
    ));
    assert_eq!(
        decl.metadata.scheme.as_ref().map(|s| s.as_str()),
        Some("Bearer ")
    );

    // The issuer is a placeholder synthesized by the macro at schema-build
    // time (origin = "plexus", method path falls back to "plexus.login"
    // because "login" alone is not a valid MethodPath).
    assert_eq!(
        decl.metadata.issuer.origin.as_str(),
        "plexus",
        "placeholder origin"
    );
    assert_eq!(
        decl.metadata.issuer.method.as_str(),
        "plexus.login",
        "method-path fallback to plexus.<name>"
    );
}
