//! AUTHZ-CRED-MACRO-1 acceptance criterion #6: unknown `kind` value produces
//! a compile error listing the valid forms.

use plexus_auth_core::Credential;

#[derive(plexus_macros::Credentials)]
pub struct Bad {
    #[credential(
        kind = "berear",
        attach_as = "header:authorization",
    )]
    pub session: Credential<String>,
}

fn main() {}
