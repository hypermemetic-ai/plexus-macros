//! AUTHZ-CRED-MACRO-1 acceptance criterion #6: missing `kind` clause is a
//! compile error citing the missing clause.

use plexus_auth_core::Credential;

#[derive(plexus_macros::Credentials)]
pub struct Bad {
    #[credential(attach_as = "header:authorization")]
    pub session: Credential<String>,
}

fn main() {}
