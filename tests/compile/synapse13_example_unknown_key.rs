//! SYNAPSE-13 compile-fail: example(...) takes note and params only.

#[derive(Clone)]
struct T;

#[plexus_macros::activation(namespace = "t", crate_path = "plexus_core")]
impl T {
    #[plexus_macros::method(example(params = r#"{"id": "MNG-1"}"#, title = "x"))]
    async fn set_status(&self, id: String, status: String) -> impl futures::stream::Stream<Item = String> + Send + 'static {
        async_stream::stream! { yield format!("{id} {status}"); }
    }
}

fn main() {}
