//! MACRO-13 (`always`): an activation written before MACRO-13 — a type the
//! name rule cannot name, its `namespace` given — still builds as it did,
//! with a deprecation warning asking for the rename (an error here, under
//! `deny`, so the fixture can show it).
#![deny(deprecated)]

use futures::Stream;

#[derive(Clone)]
pub struct Echo;

#[plexus_macros::activation(namespace = "echo")]
impl Echo {
    #[plexus_macros::method]
    async fn say(&self, text: String) -> impl Stream<Item = String> + Send + 'static {
        futures::stream::iter([text])
    }
}

fn main() {}
