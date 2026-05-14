//! AUTHZ-CRED-MACRO-1 acceptance criterion #6: `attach_as` value that doesn't
//! parse into one of the four `AttachmentSite` variants produces a compile
//! error showing the four valid forms.

use plexus_auth_core::Credential;

#[derive(plexus_macros::Credentials)]
pub struct Bad {
    #[credential(
        kind = "bearer",
        attach_as = "body:authorization",
    )]
    pub session: Credential<String>,
}

fn main() {}
