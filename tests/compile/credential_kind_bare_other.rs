//! AUTHZ-CRED-MACRO-1 acceptance criterion #8 (negative): the bare
//! `kind = "other"` form is a compile error that points at the structured
//! `kind(other = "<name>")` form.

use plexus_auth_core::Credential;

#[derive(plexus_macros::Credentials)]
pub struct Bad {
    #[credential(
        kind = "other",
        attach_as = "header:authorization",
    )]
    pub session: Credential<String>,
}

fn main() {}
