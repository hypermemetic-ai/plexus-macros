//! R-1 compile-fail (AUTHZ-CORE-2 AC #2/#9): `public` + `scope = "..."` is a
//! contradiction — a public method has no required scope to enforce.

#[derive(Clone)]
struct Conflicted;

#[plexus_macros::activation(namespace = "conflicted", crate_path = "plexus_core")]
impl Conflicted {
    #[plexus_macros::method(public, scope = "conflicted.read")]
    async fn broken(&self) -> impl futures::stream::Stream<Item = String> + Send + 'static {
        async_stream::stream! { yield "x".into(); }
    }
}

fn main() {}
