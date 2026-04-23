//! RED-6 compile-fail: `auth = "required"` rejects a method that has
//! neither `#[from_auth]` nor `#[method(public)]`.

use plexus_core::plexus::{AuthContext, PlexusError};
use plexus_macros::PlexusRequest;

#[derive(Clone, Debug)]
struct FakeUser;

#[derive(PlexusRequest)]
struct Req {
    #[from_cookie("access_token")]
    auth_token: String,
}

struct StrictMissing;

impl StrictMissing {
    #[allow(dead_code)]
    async fn validate(&self, _: &AuthContext) -> Result<FakeUser, PlexusError> { Ok(FakeUser) }
}

#[plexus_macros::activation(
    namespace = "strict_missing",
    request = Req,
    auth = "required",
    crate_path = "plexus_core",
)]
impl StrictMissing {
    #[plexus_macros::method]
    async fn authed(&self, #[from_auth(self.validate)] _u: FakeUser)
        -> impl futures::stream::Stream<Item = String> + Send + 'static
    { async_stream::stream! { yield "a".into(); } }

    // Violation: no from_auth, no public acknowledgment, posture = required.
    #[plexus_macros::method]
    async fn leak(&self) -> impl futures::stream::Stream<Item = String> + Send + 'static {
        async_stream::stream! { yield "!".into(); }
    }
}

fn main() {}
