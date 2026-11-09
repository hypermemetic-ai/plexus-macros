//! CHILD-4 matrix row 3: a search method whose parameter list doesn't match
//! `(&self, query: &str)` must fail to compile with a signature-mismatch
//! error. Here the method takes `u32` instead of `&str`.

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

    #[plexus_macros::child(search = find_by_id)]
    fn planet(&self, _name: &str) -> Option<Leaf> {
        Some(Leaf)
    }

    // Wrong parameter type: expected `&str`.
    fn find_by_id(&self, _id: u32) -> impl Stream<Item = String> + Send + '_ {
        stream! { yield "nope".to_string(); }
    }
}

fn main() {}
