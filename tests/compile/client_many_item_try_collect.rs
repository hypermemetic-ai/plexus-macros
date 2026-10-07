//! RETURNS-9 positive fixture: a many-item method's client is a plain
//! `TryStream`, so `.try_collect::<Vec<_>>()` is what MACRO-2's `m_all` was;
//! a one-item method's is an `async fn` whose output is the item.

use futures::{Stream, TryStreamExt};
use plexus_client::{ClientError, MemoryTransport};

pub struct Counter;

#[plexus_macros::activation(client)]
impl Counter {
    #[plexus_macros::method]
    async fn count(&self, to: u32) -> impl Stream<Item = u32> + Send + 'static {
        futures::stream::iter(1..=to)
    }

    #[plexus_macros::method]
    async fn total(&self) -> Result<u32, String> {
        Ok(6)
    }
}

async fn use_it(client: &CounterClient<MemoryTransport>) -> Result<(), ClientError> {
    let ticks: Vec<u32> = client.count(3).try_collect::<Vec<_>>().await?;
    let total: Result<u32, String> = client.total().await?;
    let _ = (ticks, total);
    Ok(())
}

fn main() {
    let _ = use_it;
}
