//! MACRO-2 negative fixture: `client` on an activation with a method that
//! does not return `impl Stream<Item = ..>`.
//!
//! Such a method has no event type, so the generated client has nothing to
//! give a caller. Silently omitting it would recreate exactly the drift this
//! ticket removes — the consumer would find the method missing from the
//! client and hand-write it again.

use futures::Stream;

pub struct Thing;

#[plexus_macros::activation(namespace = "thing", client)]
impl Thing {
    #[plexus_macros::method]
    async fn ping(&self) -> impl Stream<Item = String> + Send + 'static {
        futures::stream::empty()
    }

    #[plexus_macros::method]
    async fn count(&self) -> u64 {
        0
    }
}

fn main() {}
