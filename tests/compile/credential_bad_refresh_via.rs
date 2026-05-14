//! AUTHZ-CRED-MACRO-1 acceptance criterion #6: `refresh_via` value that
//! doesn't parse as a `MethodPath` produces a compile error citing the
//! path syntax.

use plexus_auth_core::Credential;

#[derive(plexus_macros::Credentials)]
pub struct Bad {
    #[credential(
        kind = "oauth_access",
        attach_as = "header:authorization",
        refresh_via = "Auth.Refresh",
    )]
    pub session: Credential<String>,
}

fn main() {}
