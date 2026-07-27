//! PLX-93 / M1-T6, on the path that ships.
//!
//! PLX-73 M1-T6 says a `#[method(unknwon = 1)]` is "a compile error naming the
//! key (trybuild golden)". PLX-79 built exactly that — for `method_ir!`, the
//! standalone macro. `#[activation]`, which is what every live service writes,
//! parsed its `#[method(...)]` list in `Mode::Legacy`, which had NO allowlist:
//! the typo below compiled silently and the key was dropped on the floor.
//!
//! This fixture is the same assertion on the production path. It must fail, and
//! the failure must name `unknwon`.

#[derive(Clone)]
struct Typo;

#[plexus_macros::activation(
    namespace = "typo",
    version = "1.0.0",
    crate_path = "plexus_core"
)]
impl Typo {
    #[plexus_macros::method(public, unknwon = 1)]
    async fn ping(&self) -> impl futures::stream::Stream<Item = String> + Send + 'static {
        async_stream::stream! { yield "pong".to_string(); }
    }
}

fn main() {}
