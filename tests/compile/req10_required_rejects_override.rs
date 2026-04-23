//! REQ-10 compile-fail: activation with `required = [auth_token]` + method
//! using `request = ()` must fail at macro expansion with an error naming
//! the locked field.

use plexus_macros::PlexusRequest;

#[derive(PlexusRequest)]
struct RequestFixture {
    #[from_cookie("access_token")]
    auth_token: String,
}

struct LockedHub;

#[plexus_macros::activation(
    namespace = "locked",
    version = "1.0.0",
    description = "REQ-10 locked",
    request = RequestFixture,
    required(auth_token),
    crate_path = "plexus_core",
)]
impl LockedHub {
    /// This method tries to drop auth_token via request = (), which is locked.
    #[plexus_macros::method(description = "should fail", request = ())]
    async fn list(&self) -> impl futures::stream::Stream<Item = String> + Send + 'static {
        async_stream::stream! { yield "x".into(); }
    }
}

fn main() {}
