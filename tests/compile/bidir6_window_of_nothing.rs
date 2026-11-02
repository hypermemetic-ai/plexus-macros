//! BIDIR-6 compile-fail: a window of 0 lets nothing in.
use futures::stream::Stream;
use plexus_core::plexus::Input;

#[derive(Clone)]
struct Ingest;

#[plexus_macros::activation(namespace = "ingest", crate_path = "plexus_core")]
impl Ingest {
    #[plexus_macros::method(input_window = 0)]
    async fn tokens(&self, tokens: Input<u32>) -> impl Stream<Item = String> + Send + 'static {
        let _ = tokens;
        async_stream::stream! { yield String::new(); }
    }
}

fn main() {}
