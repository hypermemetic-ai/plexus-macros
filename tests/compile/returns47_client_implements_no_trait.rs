//! RETURNS-47: a named trait is the activation's contract; the client has
//! the same methods, answering `Result`s, and implements no trait.

use futures::Stream;
use plexus_client::MemoryTransport;
use std::sync::Arc;

#[derive(Clone)]
pub struct Tickets;

#[plexus_macros::activation(trait = "Tickets2")]
impl Tickets {
    /// Every id.
    pub async fn ids(&self, to: u32) -> impl Stream<Item = u32> + Send + 'static {
        futures::stream::iter(1..=to)
    }
}

fn main() {
    fn is_tickets2<T: Tickets2>() {}
    is_tickets2::<Tickets>();
    is_tickets2::<Tickets2Client<Arc<MemoryTransport>>>();
}
