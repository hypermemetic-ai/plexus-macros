//! SYNAPSE-13 compile-fail: example params must be JSON.

#[derive(Clone)]
struct T;

#[plexus_macros::activation(namespace = "t", crate_path = "plexus_core")]
impl T {
    #[plexus_macros::method(example(note = "n", params = "{id: MNG-1}"))]
    async fn set_status(&self, id: String, status: String) -> impl futures::stream::Stream<Item = String> + Send + 'static {
        async_stream::stream! { yield format!("{id} {status}"); }
    }
}

fn main() {}
