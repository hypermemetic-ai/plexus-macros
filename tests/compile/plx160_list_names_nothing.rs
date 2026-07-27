//! PLX-160 c3 — `list = "…"` naming a method the impl block does not define.
//!
//! Before PLX-160 this was invisible in both directions: a `list =` on a
//! **dynamic** child was discarded before it reached the IR, so a typo produced
//! no diagnostic and no wire effect. Now the name is written into the served
//! document as RFC 002 §5.1's mandatory enumeration method, so an unresolvable
//! one advertises a method the backend does not have — which is the fabrication
//! §5.2 exists to forbid. It must be a compile error naming the method.

use futures::Stream;
use async_stream::stream;

#[derive(Clone)]
struct Leaf;

#[plexus_macros::activation(namespace = "leaf", version = "1.0.0", crate_path = "plexus_core")]
impl Leaf {
    #[plexus_macros::method]
    async fn ping(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "pong".into(); }
    }
}

#[derive(Clone)]
struct Typo;

#[plexus_macros::activation(namespace = "typo", version = "1.0.0", crate_path = "plexus_core")]
impl Typo {
    #[plexus_macros::method]
    async fn ping(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "pong".into(); }
    }

    /// The sibling is spelled `session_ids`; this names `session_idz`.
    #[plexus_macros::child(list = "session_idz")]
    fn session(&self, _id: &str) -> Option<Leaf> {
        Some(Leaf)
    }

    fn session_ids(&self) -> impl Stream<Item = String> + Send + '_ {
        stream! { yield "a".to_string(); }
    }
}

fn main() {}
