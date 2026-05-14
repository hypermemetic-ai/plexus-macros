//! AUTHZ-CRED-MACRO-1 acceptance criterion #2:
//!
//! A struct with multiple `#[plexus::credential(...)]` fields registers one
//! entry per field in stable declaration order. Models the OAuth-style
//! `TokenSet { access, refresh }` return shape.

use plexus_auth_core::{
    AttachmentSite, CookieName, Credential, CredentialKind, CredentialKindName, CredentialScheme,
    HeaderName, MethodPath, ParamName, Scope,
};

#[allow(dead_code)]
pub struct CredentialFieldMarker {
    pub variant: Option<&'static str>,
    pub field: &'static str,
    pub kind: CredentialKind,
    pub attach_as: AttachmentSite,
    pub scheme: Option<CredentialScheme>,
    pub scopes: Vec<Scope>,
    pub refresh_via: Option<MethodPath>,
    pub revoke_via: Option<MethodPath>,
}

#[derive(plexus_macros::Credentials)]
#[allow(dead_code)]
pub struct TokenSet {
    #[credential(
        kind = "oauth_access",
        attach_as = "header:authorization",
        scheme = "Bearer ",
        scopes = ["read", "write"],
        refresh_via = "auth.refresh",
        revoke_via = "auth.logout",
    )]
    pub access: Credential<String>,

    #[credential(
        kind = "oauth_refresh",
        attach_as = "in_rpc_param:refresh_token",
        scopes = ["refresh"],
    )]
    pub refresh: Credential<String>,

    pub expires_in: u64,
}

fn main() {
    let entries = __plexus_credential_marker_for_TokenSet();
    assert_eq!(entries.len(), 2);

    // Order is stable — declaration order.
    assert_eq!(entries[0].field, "access");
    assert!(matches!(entries[0].kind, CredentialKind::OauthAccess));
    assert!(matches!(
        entries[0].attach_as,
        AttachmentSite::Header { ref name } if name.as_str() == "authorization"
    ));
    assert_eq!(
        entries[0].scheme.as_ref().map(|s| s.as_str()),
        Some("Bearer ")
    );
    assert_eq!(entries[0].scopes.len(), 2);
    assert_eq!(entries[0].scopes[0].as_str(), "read");
    assert_eq!(entries[0].scopes[1].as_str(), "write");
    assert_eq!(
        entries[0].refresh_via.as_ref().map(|m| m.as_str()),
        Some("auth.refresh")
    );
    assert_eq!(
        entries[0].revoke_via.as_ref().map(|m| m.as_str()),
        Some("auth.logout")
    );

    assert_eq!(entries[1].field, "refresh");
    assert!(matches!(entries[1].kind, CredentialKind::OauthRefresh));
    assert!(matches!(
        entries[1].attach_as,
        AttachmentSite::InRpcParam { ref param } if param.as_str() == "refresh_token"
    ));
    assert!(entries[1].scheme.is_none());
    assert_eq!(entries[1].scopes.len(), 1);
    assert_eq!(entries[1].scopes[0].as_str(), "refresh");

    // Implicit sensitivity covers both fields.
    let sensitive = __plexus_sensitive_fields_for_TokenSet();
    assert_eq!(sensitive.len(), 2);
    assert_eq!(sensitive[0], (None, "access"));
    assert_eq!(sensitive[1], (None, "refresh"));

    // Silence unused-import warnings on names the macro pulls in.
    let _ = CookieName::try_new("plexus_session").unwrap();
    let _ = CredentialKindName::new("custom");
    let _ = ParamName::new("ignored");
    let _ = HeaderName::try_new("authorization").unwrap();
}
