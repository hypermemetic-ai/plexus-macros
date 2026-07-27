//! PLX-147 — a misspelled `#[activation(...)]` key is a compile error.
//!
//! PLX-113's compile probe wrote exactly this attribute and it compiled clean,
//! dropping all three arguments: `HubMethodsAttrs::parse` matched the keys it
//! knew and ignored the rest. So a typo'd `backend_name` produced a
//! non-declaring document while leaving the author with the exact §3.3 advisory
//! they believed they had removed — and the same crack silently disarmed
//! `auth_posture = "mixed"` in plexus-idp and four hubs in plexus-trak.
//!
//! Each unknown key must be named, and the accepted set listed, the same shape
//! `#[method(...)]` has given since PLX-93.

use async_stream::stream;
use futures::stream::Stream;

#[derive(Clone)]
struct Typos;

#[plexus_macros::activation(
    namespace = "typos",
    version = "1.0.0",
    crate_path = "plexus_core",
    bakcend_name = "oops"
)]
impl Typos {
    #[plexus_macros::method]
    async fn ping(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "pong".into(); }
    }
}

fn main() {}
