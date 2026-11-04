//! RETURNS-46 negative fixture: a dynamic `#[child]` whose type has no
//! client (a legacy name, as plexus-substrate's `SessionChild`). The parent's
//! aggregate and accessor need the child's client, so the error says what to
//! do: give it one, or mark the child `#[child(opaque)]`.

use futures::Stream;

#[derive(Clone)]
pub struct HostActivation;

#[plexus_macros::activation(crate_path = "plexus_core")]
impl HostActivation {
    /// ticks
    #[plexus_macros::method]
    async fn tick(&self) -> impl Stream<Item = String> + Send + 'static {
        futures::stream::empty()
    }

    /// a session by id
    #[plexus_macros::child]
    fn session(&self, id: &str) -> Option<SessionChild> {
        Some(SessionChild { id: id.to_string() })
    }
}

#[derive(Clone)]
pub struct SessionChild {
    id: String,
}

#[allow(deprecated)]
#[plexus_macros::activation(namespace = "session", crate_path = "plexus_core")]
impl SessionChild {
    #[plexus_macros::method]
    async fn hello(&self) -> impl Stream<Item = String> + Send + 'static {
        futures::stream::iter(vec![self.id.clone()])
    }
}

fn main() {}
