//! RETURNS-36: `#[method(streaming)]` on a stream form still compiles (echo-rs
//! builds on 0.7.0 and 0.8 unchanged), with a deprecation warning — made an
//! error here by `deny(deprecated)` so the test can see it.
#![deny(deprecated)]

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

fn main() {}
