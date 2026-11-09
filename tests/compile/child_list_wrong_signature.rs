//! CHILD-4 acceptance criterion 7: `#[child(list = wrongly_typed)]` that
//! names a method whose return element type is not `String` (here `u32`)
//! must fail to compile with a signature-mismatch error.

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

    #[plexus_macros::child(list = wrongly_typed)]
    fn planet(&self, _name: &str) -> Option<Leaf> {
        Some(Leaf)
    }

    // Wrong element type: macro accepts only `Stream<Item = String>`.
    fn wrongly_typed(&self) -> impl Stream<Item = u32> + Send + '_ {
        stream! { yield 42u32; }
    }
}

fn main() {}
