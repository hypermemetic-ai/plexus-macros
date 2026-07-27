//! PLX-147 — the live casualty's exact spelling, refused by name.
//!
//! `plexus-idp/src/activation.rs` wrote `auth_posture = "mixed"` and the parser
//! reads `auth`, so the `identity` activation carried NO declared RED-6
//! authorization posture while its source read as though it did. The spelling
//! was not invented: the macro's own RED-2 diagnostic told authors to write it.
//!
//! `auth_posture` is deliberately NOT accepted as an alias. There is one
//! spelling for an authorization declaration, and the near-miss gets an error
//! that names the right one.

use async_stream::stream;
use futures::stream::Stream;

#[derive(Clone)]
struct Postured;

#[plexus_macros::activation(
    namespace = "postured",
    version = "1.0.0",
    crate_path = "plexus_core",
    auth_posture = "mixed"
)]
impl Postured {
    #[plexus_macros::method]
    async fn ping(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "pong".into(); }
    }
}

fn main() {}
