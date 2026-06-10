//! R-1 compile-pass: every valid scope/public/posture combination compiles —
//! single scope, multi scope (conjunction), wildcard scopes, public, and a
//! declared activation posture alongside per-method authz args.

use futures::stream::Stream;

#[derive(Clone)]
struct Combos;

#[plexus_macros::activation(
    namespace = "combos",
    auth = "mixed",
    crate_path = "plexus_core"
)]
impl Combos {
    #[plexus_macros::method(scope = "combos.read")]
    async fn single(&self) -> impl Stream<Item = String> + Send + 'static {
        async_stream::stream! { yield "s".into(); }
    }

    #[plexus_macros::method(scope = "combos.read", scope = "combos.write")]
    async fn conjunction(&self) -> impl Stream<Item = String> + Send + 'static {
        async_stream::stream! { yield "c".into(); }
    }

    #[plexus_macros::method(scope = "combos.*")]
    async fn segment_wildcard(&self) -> impl Stream<Item = String> + Send + 'static {
        async_stream::stream! { yield "sw".into(); }
    }

    #[plexus_macros::method(scope = "*")]
    async fn bare_wildcard(&self) -> impl Stream<Item = String> + Send + 'static {
        async_stream::stream! { yield "bw".into(); }
    }

    #[plexus_macros::method(public)]
    async fn open(&self) -> impl Stream<Item = String> + Send + 'static {
        async_stream::stream! { yield "o".into(); }
    }

    #[plexus_macros::method]
    async fn implicit(&self) -> impl Stream<Item = String> + Send + 'static {
        async_stream::stream! { yield "i".into(); }
    }
}

fn main() {}
