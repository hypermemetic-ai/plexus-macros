//! PLX-147 — the second half of the class: a RECOGNIZED key in a form the
//! parser does not read.
//!
//! `auth = mixed` (no quotes) matched no arm and was dropped exactly as an
//! unknown key was, so closing only the unknown-key half would leave an
//! authorization declaration that still silently vanishes. The error names the
//! key and the form it needs.

use async_stream::stream;
use futures::stream::Stream;

#[derive(Clone)]
struct Unquoted;

#[plexus_macros::activation(
    namespace = "unquoted",
    version = "1.0.0",
    crate_path = "plexus_core",
    auth = mixed
)]
impl Unquoted {
    #[plexus_macros::method]
    async fn ping(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "pong".into(); }
    }
}

fn main() {}
