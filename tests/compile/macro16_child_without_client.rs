//! MACRO-16 negative fixture: a parent with `client` reaches a static
//! `#[child]` through `InputsClient`, so the child must be declared with
//! `client` too. When it is not, the name does not resolve, and rustc points
//! at the accessor's return type.

use futures::Stream;

#[derive(Clone)]
pub struct Tickets;

#[plexus_macros::activation(namespace = "tickets", client)]
impl Tickets {
    #[plexus_macros::method]
    async fn get(&self, id: String) -> impl Stream<Item = String> + Send + 'static {
        let _ = id;
        futures::stream::empty()
    }

    #[plexus_macros::child]
    fn inputs(&self) -> Inputs {
        Inputs
    }
}

#[derive(Clone)]
pub struct Inputs;

// No `client`: there is no `InputsClient` for `TicketsClient::inputs` to return.
#[plexus_macros::activation(namespace = "inputs")]
impl Inputs {
    #[plexus_macros::method]
    async fn answer(&self, ticket: String) -> impl Stream<Item = String> + Send + 'static {
        let _ = ticket;
        futures::stream::empty()
    }
}

fn main() {}
