//! AUTH-2/AUTH-3 compile-fail: `reach: Reach` comes first, or right after
//! `auth` — the framework's arguments precede the RPC params.

use plexus_core::plexus::Reach;

#[derive(Clone)]
struct T;

#[plexus_macros::activation(namespace = "t", crate_path = "plexus_core")]
impl T {
    #[plexus_macros::method]
    async fn list(&self, filter: String, reach: Reach) -> impl futures::stream::Stream<Item = String> + Send + 'static {
        let _ = reach;
        async_stream::stream! { yield filter; }
    }
}

fn main() {}
