//! RETURNS-36: `#[method(streaming)]` where the signature agrees compiles,
//! and the method yields many items, as without it.

#[derive(Clone)]
pub struct TickerActivation;

#[plexus_macros::activation]
impl TickerActivation {
    /// Ticks.
    #[plexus_macros::method(streaming)]
    async fn tick(&self) -> impl futures::Stream<Item = String> + Send + 'static {
        futures::stream::empty()
    }
}

fn main() {
    use plexus_core::plexus::{Activation, Cardinality};
    let s = TickerActivation.plugin_schema();
    assert_eq!(s.methods[0].cardinality, Cardinality::Many);
}
