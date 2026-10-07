//! RETURNS-8 negative fixture: a `BoxStream` return is not one of the six
//! signature forms; the error lists them.

pub struct Ticker;

#[plexus_macros::activation]
impl Ticker {
    /// Ticks.
    #[plexus_macros::method]
    async fn tick(&self) -> futures::stream::BoxStream<'static, u64> {
        Box::pin(futures::stream::empty())
    }
}

fn main() {}
