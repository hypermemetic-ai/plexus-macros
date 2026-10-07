//! RETURNS-9 negative fixture: a one-item method's client has no
//! `.with_progress()`.
//!
//! A one-item method (`-> T` / `-> Result<T, E>`) cannot report progress
//! (RETURNS-1 decision 10), so its client method is a plain `async fn`
//! returning the item — there is no stream to opt in on.

use plexus_client::MemoryTransport;

pub struct Pinger;

#[plexus_macros::activation(client)]
impl Pinger {
    #[plexus_macros::method]
    async fn ping(&self) -> String {
        "pong".into()
    }
}

fn main() {
    let client = PingerClient::new(MemoryTransport::new());
    let _ = client.ping().with_progress();
}
