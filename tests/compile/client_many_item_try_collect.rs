//! RETURNS-9 positive fixture: a many-item method's client is a plain
//! `TryStream`, so `.try_collect::<Vec<_>>()` is what MACRO-2's `m_all` was;
//! a one-item method's is an `async fn` whose output is the item.

use futures::{Stream, TryStreamExt};
use plexus_client::{ClientError, MemoryTransport};

#[derive(Clone)]
pub struct CounterActivation;

#[plexus_macros::activation]
impl CounterActivation {
    #[plexus_macros::method]
    async fn count(&self, to: u32) -> impl Stream<Item = String> + Send + 'static {
        futures::stream::iter((1..=to).map(|n| n.to_string()))
    }

    #[plexus_macros::method]
    async fn total(&self) -> Result<u32, String> {
        Ok(6)
    }
}

async fn use_it(client: &CounterClient<MemoryTransport>) -> Result<(), ClientError> {
    let ticks: Vec<String> = client.count(3).try_collect::<Vec<_>>().await?;
    let total: Result<u32, String> = client.total().await?;
    let _ = (ticks, total);
    Ok(())
}

fn main() {
    let _ = use_it;
}
