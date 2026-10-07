//! RETURNS-8 negative fixture: `#[method(streaming)]` is removed — a
//! method's cardinality comes from its return type.

pub struct Ticker;

#[plexus_macros::activation]
impl Ticker {
    /// Ticks.
    #[plexus_macros::method(streaming)]
    async fn tick(&self) -> impl futures::Stream<Item = u64> + Send + 'static {
        futures::stream::empty()
    }
}

fn main() {}
