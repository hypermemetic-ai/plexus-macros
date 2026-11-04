//! RETURNS-8 negative fixture: `#[method(returns(..))]` is removed — a
//! method's own return type says what it yields.

pub struct TicketsActivation;

#[plexus_macros::activation]
impl TicketsActivation {
    /// One ticket.
    #[plexus_macros::method(returns(Ticket, NotFound))]
    async fn get(&self, id: String) -> impl futures::Stream<Item = String> + Send + 'static {
        futures::stream::iter(vec![id])
    }
}

fn main() {}
