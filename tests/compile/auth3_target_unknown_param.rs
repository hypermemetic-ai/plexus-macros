//! AUTH-3 compile-fail: `target` must name one of the method's params.

#[derive(Clone)]
struct T;

#[plexus_macros::activation(namespace = "t", crate_path = "plexus_core")]
impl T {
    #[plexus_macros::method(target = "ticket")]
    async fn get(&self, id: String) -> impl futures::stream::Stream<Item = String> + Send + 'static {
        async_stream::stream! { yield id; }
    }
}

fn main() {}
