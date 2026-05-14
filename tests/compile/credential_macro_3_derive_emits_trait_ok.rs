//! AUTHZ-CRED-MACRO-3 acceptance: `#[derive(Credentials)]` emits the
//! `HasCredentialMarkers` trait impl in addition to the per-type marker
//! function (the latter is the AUTHZ-CRED-MACRO-1 contract). The trait
//! impl is what `#[plexus_macros::activation]` codegen reaches via the
//! autoref-specialized inherent path on
//! `CredentialsRegistryProbe<T: HasCredentialMarkers>`.

use plexus_auth_core::{
    AttachmentSite, Credential, CredentialFieldMarker, CredentialIssuer, CredentialKind,
    CredentialKindName, CredentialMetadata, CredentialMinter, CredentialScheme, CookieName,
    CredentialsRegistryProbe, HasCredentialMarkers, HeaderName, MethodPath, Origin, ParamName,
    Scope,
};

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
    // The derive emits BOTH the per-type registry function (AUTHZ-CRED-MACRO-1
    // contract) AND the HasCredentialMarkers trait impl (AUTHZ-CRED-MACRO-3
    // contract). Both must produce the same slice.
    let from_fn = __plexus_credential_marker_for_LoginEvent();
    let from_trait = <LoginEvent as HasCredentialMarkers>::credential_markers();

    assert_eq!(
        from_fn.len(),
        from_trait.len(),
        "fn and trait return identical-length slices"
    );
    assert_eq!(from_fn.len(), 1);
    assert_eq!(from_trait[0].field, "session");
    assert!(matches!(from_trait[0].kind, CredentialKind::Bearer));

    // The trait impl wires up to the autoref-specialized inherent path on
    // CredentialsRegistryProbe<LoginEvent>. The same call shape used by
    // #[plexus_macros::method] codegen:
    let probe = CredentialsRegistryProbe::<LoginEvent>::new();
    let m = probe.marker_slice();
    assert_eq!(m.len(), 1);
    assert_eq!(m[0].field, "session");

    // Suppress unused-import warnings on names exercised only by
    // macro-emitted code paths.
    let _ = CredentialFieldMarker::new(
        None,
        "x",
        CredentialKind::Bearer,
        AttachmentSite::Header {
            name: HeaderName::try_new("authorization").unwrap(),
        },
        None,
        vec![],
        None,
        None,
    );
    let _ = CredentialKindName::new("test");
    let _ = ParamName::new("test");
    let _ = CookieName::try_new("test").unwrap();
    let _ = CredentialIssuer::new(
        Origin::new("ws://localhost:4444"),
        MethodPath::try_new("auth.login").unwrap(),
    );
    let _: fn() -> CredentialMetadata = || {
        CredentialMetadata::new(
            CredentialKind::Bearer,
            AttachmentSite::Header {
                name: HeaderName::try_new("authorization").unwrap(),
            },
            Some(CredentialScheme::new("Bearer ")),
            vec![Scope::new("placeholder")],
            None,
            None,
            None,
            CredentialIssuer::new(
                Origin::new("ws://localhost:4444"),
                MethodPath::try_new("auth.login").unwrap(),
            ),
        )
    };
    let _ = CredentialMinter::clone;
}
