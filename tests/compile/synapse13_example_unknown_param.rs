//! SYNAPSE-13 compile-fail: example params name only the method's params.

#[derive(Clone)]
struct T;

#[plexus_macros::activation(namespace = "t", crate_path = "plexus_core")]
impl T {
    #[plexus_macros::method(example(params = r#"{"id": "MNG-1", "state": "Ready"}"#))]
    async fn set_status(&self, id: String, status: String) -> impl futures::stream::Stream<Item = String> + Send + 'static {
        async_stream::stream! { yield format!("{id} {status}"); }
    }
}

fn main() {}
