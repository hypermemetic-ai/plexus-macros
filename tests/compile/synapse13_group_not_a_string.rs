//! SYNAPSE-13 compile-fail: group is a string.

#[derive(Clone)]
struct T;

#[plexus_macros::activation(namespace = "t", crate_path = "plexus_core")]
impl T {
    #[plexus_macros::method(group = write)]
    async fn set_status(&self, id: String, status: String) -> impl futures::stream::Stream<Item = String> + Send + 'static {
        async_stream::stream! { yield format!("{id} {status}"); }
    }
}

fn main() {}
