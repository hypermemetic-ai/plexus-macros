//! AUTHZ-CRED-MACRO-1 acceptance criterion #5: `#[plexus::credential(...)]`
//! on a field whose type is not `Credential<T>` is a compile error.

#[derive(plexus_macros::Credentials)]
pub struct Bad {
    #[credential(
        kind = "bearer",
        attach_as = "header:authorization",
    )]
    pub session: String,
}

fn main() {}
