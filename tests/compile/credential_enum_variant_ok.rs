//! AUTHZ-CRED-MACRO-1 acceptance criterion #3:
//!
//! An enum with a `#[plexus::credential(...)]` field on one variant registers
//! one entry tagged with the variant name. Entries on variants without any
//! credential-bearing field are NOT registered.

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
pub enum LoginOutcome {
    /// A successful login carries a session credential.
    Issued {
        user_id: String,
        #[credential(
            kind = "bearer",
            attach_as = "header:authorization",
            scheme = "Bearer ",
        )]
        session: Credential<String>,
    },

    /// A login failure carries no credential.
    Denied { reason: String },

    /// Unit variants compile cleanly.
    PendingTwoFactor,
}

fn main() {
    let entries = __plexus_credential_marker_for_LoginOutcome();
    // Only the `Issued` variant contributes an entry.
    assert_eq!(entries.len(), 1);
    let e = &entries[0];
    assert_eq!(e.variant, Some("Issued"));
    assert_eq!(e.field, "session");
    assert!(matches!(e.kind, CredentialKind::Bearer));

    // The sensitive registry mirrors the credential registry, tagged with
    // the variant name.
    let sensitive = __plexus_sensitive_fields_for_LoginOutcome();
    assert_eq!(sensitive.len(), 1);
    assert_eq!(sensitive[0], (Some("Issued"), "session"));

    // Silence unused-import warnings on names the macro pulls in.
    let _ = CookieName::try_new("plexus_session").unwrap();
    let _ = CredentialKindName::new("custom");
    let _ = ParamName::new("ignored");
    let _ = HeaderName::try_new("authorization").unwrap();
}
