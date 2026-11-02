//! BIDIR-6 compile-fail: `Input` is top-level, by value.
use futures::stream::Stream;
use plexus_core::plexus::Input;

#[derive(Clone)]
struct Ingest;

#[plexus_macros::activation(namespace = "ingest", crate_path = "plexus_core")]
impl Ingest {
    #[plexus_macros::method]
    async fn tokens(&self, tokens: Option<Input<u32>>) -> impl Stream<Item = String> + Send + 'static {
        let _ = tokens;
        async_stream::stream! { yield String::new(); }
    }
}

fn main() {}
