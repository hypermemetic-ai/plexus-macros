//! AUTHZ-CRED-MACRO-1 acceptance criterion #7:
//!
//! A struct with no `#[plexus::credential(...)]` fields produces an empty
//! registry. The macro emits valid registry helpers with no entries; the
//! pre-existing struct shape is unchanged.

use plexus_auth_core::{AttachmentSite, CredentialKind, CredentialScheme, MethodPath, Scope};

// Local stand-in for the CredentialFieldMarker registry type. (See run-notes.)
// Unused here because no fields are credential-tagged; defined to keep the
// fixture's import surface symmetric with the other compile-pass fixtures.

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
pub struct PlainResponse {
    pub user_id: String,
    pub message: String,
}

fn main() {
    let entries = __plexus_credential_marker_for_PlainResponse();
    assert!(entries.is_empty(), "no credentials declared");
    let sensitive = __plexus_sensitive_fields_for_PlainResponse();
    assert!(sensitive.is_empty(), "no sensitive fields declared");
}
