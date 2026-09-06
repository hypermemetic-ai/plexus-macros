//! MACRO-2 negative fixture: `server_cfg` without `client`.
//!
//! `server_cfg` suppresses the server half of the expansion. Without
//! `client` there is nothing left, so the attribute would silently emit an
//! empty expansion — the author would get "no such method" errors far from
//! the cause. The macro names it instead.

use futures::Stream;

pub struct Thing;

#[plexus_macros::activation(namespace = "thing", server_cfg = "feature = \"server\"")]
impl Thing {
    #[plexus_macros::method]
    async fn ping(&self) -> impl Stream<Item = String> + Send + 'static {
        futures::stream::empty()
    }
}

fn main() {}
