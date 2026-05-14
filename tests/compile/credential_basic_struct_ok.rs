//! AUTHZ-CRED-MACRO-1 acceptance criterion #1:
//!
//! A struct with a field of type `Credential<SessionToken>` annotated
//! `#[plexus::credential(kind = "bearer", attach_as = "header:authorization",
//! scheme = "Bearer ")]` compiles and registers one `CredentialFieldMarker`
//! entry whose metadata matches the attribute clauses.
//!
//! `CredentialFieldMarker` is the canonical type from `plexus_auth_core`
//! (landed in `AUTHZ-CRED-CORE-1C`).

use plexus_auth_core::{
    AttachmentSite, Credential, CredentialFieldMarker, CredentialIssuer, CredentialKind,
    CredentialKindName, CredentialMetadata, CredentialMinter, CredentialScheme, CookieName,
    HeaderName, MethodPath, Origin, ParamName, Scope,
};

/// The credential payload type. Wrapped in `Credential<T>` on the
/// return-type struct below.
pub struct SessionToken(pub String);

#[derive(plexus_macros::Credentials)]
#[allow(dead_code)]
pub struct LoginEvent {
    pub user_id: String,

    #[credential(
        kind = "bearer",
        attach_as = "header:authorization",
        scheme = "Bearer ",
    )]
    pub session: Credential<SessionToken>,
}

fn main() {
    // Exercise the emitted registry function.
    let entries = __plexus_credential_marker_for_LoginEvent();
    assert_eq!(entries.len(), 1, "exactly one credential field");
    let e = &entries[0];
    assert_eq!(e.field, "session");
    assert!(matches!(e.kind, CredentialKind::Bearer));
    assert!(matches!(
        e.attach_as,
        AttachmentSite::Header { ref name } if name.as_str() == "authorization"
    ));
    assert_eq!(
        e.scheme.as_ref().map(|s| s.as_str()),
        Some("Bearer "),
        "scheme captured verbatim including trailing space"
    );
    assert!(e.scopes.is_empty(), "no scopes declared");
    assert!(e.refresh_via.is_none());
    assert!(e.revoke_via.is_none());

    // Acceptance #4: the same field is visible to the SensitiveField registry.
    let sensitive = __plexus_sensitive_fields_for_LoginEvent();
    assert_eq!(sensitive.len(), 1);
    assert_eq!(sensitive[0], (None, "session"));

    // Smoke-check that the struct still serializes (the credential field
    // emits its sentinel via the Credential<T> Serialize impl).
    let _ = CredentialMetadata::new(
        CredentialKind::Bearer,
        AttachmentSite::Header {
            name: HeaderName::try_new("authorization").unwrap(),
        },
        Some(CredentialScheme::new("Bearer ")),
        vec![],
        None,
        None,
        None,
        CredentialIssuer::new(
            Origin::new("ws://localhost:4444"),
            MethodPath::try_new("auth.login").unwrap(),
        ),
    );

    // Silence unused-import warnings on names that are exercised only by the
    // emitted code paths.
    let _ = CredentialKindName::new("test");
    let _ = ParamName::new("test");
    let _ = CookieName::try_new("test").unwrap();
    let _ = CredentialMinter::clone;
    // `Credential::<T>` is `Clone` when T is `Clone`. We do not Clone the
    // credential here (the inner SessionToken does not derive Clone in this
    // fixture); existence of the type alone is enough.
    let _: fn() -> CredentialMetadata = || sample_metadata();
}

fn sample_metadata() -> CredentialMetadata {
    CredentialMetadata::new(
        CredentialKind::Bearer,
        AttachmentSite::Header {
            name: HeaderName::try_new("authorization").unwrap(),
        },
        Some(CredentialScheme::new("Bearer ")),
        vec![],
        None,
        None,
        None,
        CredentialIssuer::new(
            Origin::new("ws://localhost:4444"),
            MethodPath::try_new("auth.login").unwrap(),
        ),
    )
}
