//! CHILD-4 acceptance criterion 6: `#[child(list = nonexistent_method)]`
//! where the named method does not exist in the same impl must fail to
//! compile with an error containing `not found in impl`.

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

    // `nonexistent_method` is not defined anywhere in this impl block.
    #[plexus_macros::child(list = nonexistent_method)]
    fn planet(&self, _name: &str) -> Option<Leaf> {
        Some(Leaf)
    }
}

fn main() {}
