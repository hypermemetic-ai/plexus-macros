//! AUTH-3 compile-fail: a role may list only methods the activation has.

#[derive(Clone)]
struct T;

#[plexus_macros::activation(namespace = "t", crate_path = "plexus_core", roles(viewer = ["get", "lst"]))]
impl T {
    #[plexus_macros::method]
    async fn get(&self) -> impl futures::stream::Stream<Item = String> + Send + 'static {
        async_stream::stream! { yield "x".into(); }
    }

    #[plexus_macros::method]
    async fn list(&self) -> impl futures::stream::Stream<Item = String> + Send + 'static {
        async_stream::stream! { yield "x".into(); }
    }
}

fn main() {}
