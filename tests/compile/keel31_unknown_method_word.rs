//! KEEL-31 compile-fail: a word `#[method(...)]` does not take is an error
//! naming it, never dropped (an older macro dropped `everywhere`).

#[derive(Clone)]
struct T;

#[plexus_macros::activation(namespace = "t", crate_path = "plexus_core")]
impl T {
    #[plexus_macros::method(everywher)]
    async fn grow(&self) -> impl futures::stream::Stream<Item = String> + Send + 'static {
        async_stream::stream! { yield String::new(); }
    }
}

fn main() {}
