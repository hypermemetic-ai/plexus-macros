//! PLX-113 / RFC 002 §3.5 — a present-but-empty root fact is not a
//! declaration.
//!
//! §3.5: "an empty string is not a value of an optional field: a decoder MUST
//! treat a present-but-empty optional as it treats an absent one, and an
//! encoder MUST NOT produce one." connectome-hs enforces exactly that as two
//! **MUST** violations — `backend_name is present but empty` and
//! `respond_method is present but empty`.
//!
//! So `backend_name = ""` is strictly worse than omitting it: omission is a
//! SHOULD advisory, the empty spelling is non-conformance. It is refused at the
//! source, where the author can still fix it, rather than emitted and caught by
//! a checker nobody runs in CI.

#[derive(Clone)]
struct Empty;

#[plexus_macros::activation(
    namespace = "empty",
    version = "1.0.0",
    backend_name = "",
    respond_method = "",
    crate_path = "plexus_core"
)]
impl Empty {
    #[plexus_macros::method(public)]
    async fn ping(&self) -> impl futures::stream::Stream<Item = String> + Send + 'static {
        async_stream::stream! { yield "pong".to_string(); }
    }
}

fn main() {}
