//! R-1 compile-fail (AUTHZ-CORE-2 AC #3/#10): a scope literal that violates
//! the AUTHZ-CORE-4 grammar is rejected at macro-expansion time, pointing at
//! the literal.

#[derive(Clone)]
struct BadScope;

#[plexus_macros::activation(namespace = "bad_scope", crate_path = "plexus_core")]
impl BadScope {
    #[plexus_macros::method(scope = "INVALID-CHARS")]
    async fn broken(&self) -> impl futures::stream::Stream<Item = String> + Send + 'static {
        async_stream::stream! { yield "x".into(); }
    }
}

fn main() {}
