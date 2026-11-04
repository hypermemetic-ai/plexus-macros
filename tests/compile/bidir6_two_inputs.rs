//! BIDIR-6 compile-fail: a method takes at most one `Input`.
use futures::stream::Stream;
use plexus_core::plexus::Input;

#[derive(Clone)]
struct Ingest;

#[plexus_macros::activation(namespace = "ingest", crate_path = "plexus_core")]
impl Ingest {
    #[plexus_macros::method]
    async fn tokens(&self, a: Input<u32>, b: Input<u32>) -> impl Stream<Item = String> + Send + 'static {
        let _ = (a, b);
        async_stream::stream! { yield String::new(); }
    }
}

fn main() {}
