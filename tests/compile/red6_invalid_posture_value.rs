//! RED-6 compile-fail: `auth = "invalid-value"` is rejected by the parser
//! with a clear error naming the four valid posture values.

use plexus_macros::PlexusRequest;

#[derive(PlexusRequest)]
struct Req {
    #[from_cookie("access_token")]
    auth_token: String,
}

struct Bogus;

#[plexus_macros::activation(
    namespace = "bogus",
    request = Req,
    auth = "strict_please",
    crate_path = "plexus_core",
)]
impl Bogus {
    #[plexus_macros::method]
    async fn ping(&self) -> impl futures::stream::Stream<Item = String> + Send + 'static {
        async_stream::stream! { yield "".into(); }
    }
}

fn main() {}
