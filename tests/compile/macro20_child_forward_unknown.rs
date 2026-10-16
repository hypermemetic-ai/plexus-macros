//! MACRO-20: a forward policy the macro does not know is refused, naming
//! the spellings it takes.

use async_stream::stream;
use futures::stream::Stream;

#[derive(Clone)]
struct Leaf;

#[plexus_macros::activation(namespace = "leaf", version = "1.0.0", crate_path = "plexus_core")]
impl Leaf {
    #[plexus_macros::method]
    async fn ping(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "pong".into(); }
    }
}

#[derive(Clone)]
struct Hub;

#[plexus_macros::activation(namespace = "hub", version = "1.0.0", crate_path = "plexus_core")]
impl Hub {
    #[plexus_macros::method]
    async fn ping(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "pong".into(); }
    }

    #[plexus_macros::child(forward = "everything")]
    fn leaf(&self) -> Leaf {
        Leaf
    }
}

fn main() {}
