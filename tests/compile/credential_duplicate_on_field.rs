//! AUTHZ-CRED-MACRO-1 acceptance criterion #6: two `#[plexus::credential(...)]`
//! attributes on the same field is a compile error.

use plexus_auth_core::Credential;

#[derive(plexus_macros::Credentials)]
pub struct Bad {
    #[credential(kind = "bearer", attach_as = "header:authorization")]
    #[credential(kind = "cookie", attach_as = "cookie:plexus_session")]
    pub session: Credential<String>,
}

fn main() {}
