//! AUTH-3 compile-fail: `admin` is built in and needs no list.

#[derive(Clone)]
struct T;

#[plexus_macros::activation(namespace = "t", crate_path = "plexus_core", roles(admin = ["get"]))]
impl T {
    #[plexus_macros::method]
    async fn get(&self) -> impl futures::stream::Stream<Item = String> + Send + 'static {
        async_stream::stream! { yield "x".into(); }
    }
}

fn main() {}
