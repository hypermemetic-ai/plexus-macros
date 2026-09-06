//! MACRO-2 positive fixture: an activation with no `client` flag expands
//! exactly as it did before this build — no client type, no reference to
//! `plexus-client`, no new dependency.
//!
//! The compile-pass is the assertion: `NoClientThingClient` is never named,
//! and this crate would not resolve it if the macro had emitted it under a
//! different opt-in rule.

use futures::Stream;

pub struct NoClientThing;

#[plexus_macros::activation(namespace = "no_client_thing")]
impl NoClientThing {
    #[plexus_macros::method]
    async fn ping(&self) -> impl Stream<Item = String> + Send + 'static {
        futures::stream::empty()
    }
}

fn main() {
    // The server half is here, as always.
    use plexus_core::plexus::Activation;
    fn assert_activation<A: Activation>() {}
    assert_activation::<NoClientThing>();
}
