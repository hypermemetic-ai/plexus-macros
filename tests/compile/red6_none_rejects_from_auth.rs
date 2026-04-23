//! RED-6 compile-fail: `auth = "none"` rejects any method with `#[from_auth]`.

use plexus_core::plexus::{AuthContext, PlexusError};
use plexus_macros::PlexusRequest;

#[derive(Clone, Debug)]
struct FakeUser;

#[derive(PlexusRequest)]
struct Req {
    #[from_cookie("access_token")]
    auth_token: String,
}

struct PublicWithAuth;

impl PublicWithAuth {
    #[allow(dead_code)]
    async fn validate(&self, _: &AuthContext) -> Result<FakeUser, PlexusError> { Ok(FakeUser) }
}

#[plexus_macros::activation(
    namespace = "public_with_auth",
    request = Req,
    auth = "none",
    crate_path = "plexus_core",
)]
impl PublicWithAuth {
    // Violation: posture = none but this method still uses from_auth.
    #[plexus_macros::method]
    async fn contradiction(&self, #[from_auth(self.validate)] _u: FakeUser)
        -> impl futures::stream::Stream<Item = String> + Send + 'static
    { async_stream::stream! { yield "?".into(); } }
}

fn main() {}
