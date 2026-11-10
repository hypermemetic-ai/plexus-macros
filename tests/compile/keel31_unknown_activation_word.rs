//! KEEL-31 compile-fail: a word `#[activation(...)]` does not take is an error.

#[derive(Clone)]
struct T;

#[plexus_macros::activation(namespace = "t", crate_path = "plexus_core", forward_policy = "pass_through")]
impl T {
    #[plexus_macros::method]
    async fn get(&self) -> impl futures::stream::Stream<Item = String> + Send + 'static {
        async_stream::stream! { yield String::new(); }
    }
}

fn main() {}
