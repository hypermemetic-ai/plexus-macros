//! AUTHZ-CRED-MACRO-1 acceptance criterion #7:
//!
//! A struct with no `#[plexus::credential(...)]` fields produces an empty
//! registry. The macro emits valid registry helpers with no entries; the
//! pre-existing struct shape is unchanged.

// The canonical `CredentialFieldMarker` is imported so the macro's
// `__plexus_credential_marker_for_<TypeIdent>` return-type can be named.
// Unused here because no fields are credential-tagged; kept for symmetry
// with the other compile-pass fixtures.
#[allow(unused_imports)]
use plexus_auth_core::CredentialFieldMarker;

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
