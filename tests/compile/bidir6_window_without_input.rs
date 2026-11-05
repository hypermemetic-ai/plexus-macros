//! BIDIR-6 compile-fail: `input_window` needs an `Input`.
use futures::stream::Stream;

#[derive(Clone)]
struct Ingest;

#[plexus_macros::activation(namespace = "ingest", crate_path = "plexus_core")]
impl Ingest {
    #[plexus_macros::method(input_window = 256)]
    async fn tokens(&self, run: u32) -> impl Stream<Item = String> + Send + 'static {
        async_stream::stream! { yield run.to_string(); }
    }
}

fn main() {}
