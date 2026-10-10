//! AUTH-3 compile-fail: a role may include only roles declared before it.

#[derive(Clone)]
struct T;

#[plexus_macros::activation(
    namespace = "t",
    crate_path = "plexus_core",
    roles(editor = [viewer, "set"], viewer = ["get"])
)]
impl T {
    #[plexus_macros::method]
    async fn get(&self) -> impl futures::stream::Stream<Item = String> + Send + 'static {
        async_stream::stream! { yield "x".into(); }
    }

    #[plexus_macros::method]
    async fn set(&self) -> impl futures::stream::Stream<Item = String> + Send + 'static {
        async_stream::stream! { yield "x".into(); }
    }
}

fn main() {}
