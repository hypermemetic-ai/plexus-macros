//! AUTHZ-CRED-MACRO-1 acceptance criterion #8 (positive):
//!
//! The structured `kind(other = "<name>")` escape valve compiles and produces
//! a `CredentialKind::Other { name: CredentialKindName::new("...") }`.

use plexus_auth_core::{
    AttachmentSite, Credential, CredentialFieldMarker, CredentialKind, CredentialKindName,
    CredentialScheme, HeaderName, MethodPath, Scope,
};

#[derive(plexus_macros::Credentials)]
#[allow(dead_code)]
pub struct CustomCredEvent {
    #[credential(
        kind(other = "trak.custom"),
        attach_as = "header:x-trak-token",
    )]
    pub token: Credential<String>,
}

fn main() {
    let entries = __plexus_credential_marker_for_CustomCredEvent();
    assert_eq!(entries.len(), 1);
    let e = &entries[0];
    match &e.kind {
        CredentialKind::Other { name } => {
            assert_eq!(name.as_str(), "trak.custom");
        }
        other => panic!("expected Other kind, got {:?}", other),
    }
    // Silence unused-import warnings on names the macro pulls in.
    let _ = CredentialKindName::new("warm");
    let _ = HeaderName::try_new("authorization").unwrap();
    let _ = CredentialScheme::new("Bearer ");
    let _ = MethodPath::try_new("auth.login").unwrap();
    let _ = Scope::new("x");
    let _ = AttachmentSite::Header {
        name: HeaderName::try_new("a").unwrap(),
    };
}
