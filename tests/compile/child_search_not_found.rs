//! CHILD-4 matrix row 2: `#[child(search = nonexistent_method)]` where
//! the named method does not exist in the same impl must fail with an
//! error containing `not found in impl`.

use async_stream::stream;
use futures::stream::Stream;

#[derive(Clone)]
struct Leaf;

#[plexus_macros::activation(
    namespace = "leaf",
    version = "1.0.0",
    crate_path = "plexus_core"
)]
impl Leaf {
    #[plexus_macros::method]
    async fn ping(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "pong".into(); }
    }
}

struct Solar;

#[plexus_macros::activation(
    namespace = "solar",
    version = "1.0.0",
    crate_path = "plexus_core"
)]
impl Solar {
    #[plexus_macros::method]
    async fn ping(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "pong".into(); }
    }

    #[plexus_macros::child(search = never_declared)]
    fn planet(&self, _name: &str) -> Option<Leaf> {
        Some(Leaf)
    }
}

fn main() {}
