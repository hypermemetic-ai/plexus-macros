//! R-1 compile-fail: `scope = ...` expects a string literal.

#[derive(Clone)]
struct NotAString;

#[plexus_macros::activation(namespace = "not_a_string", crate_path = "plexus_core")]
impl NotAString {
    #[plexus_macros::method(scope = 42)]
    async fn broken(&self) -> impl futures::stream::Stream<Item = String> + Send + 'static {
        async_stream::stream! { yield "x".into(); }
    }
}

fn main() {}
