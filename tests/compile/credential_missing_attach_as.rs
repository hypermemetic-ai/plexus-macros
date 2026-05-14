//! AUTHZ-CRED-MACRO-1 acceptance criterion #6: missing `attach_as` clause is
//! a compile error citing the missing clause.

use plexus_auth_core::Credential;

#[derive(plexus_macros::Credentials)]
pub struct Bad {
    #[credential(kind = "bearer")]
    pub session: Credential<String>,
}

fn main() {}
