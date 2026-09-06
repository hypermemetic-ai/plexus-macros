//! MACRO-2 fixture — an activation crate that is reachable from wasm32.
//!
//! One crate, two builds:
//!
//! | Build | Gets |
//! |---|---|
//! | `tickets-api` (default) | the activation, the `Activation` impl, the jsonrpsee trait — **and** the client |
//! | `tickets-api` with `default-features = false` | the types and the generated `TicketsClient` — nothing else |
//!
//! The second is the wasm-safe artifact MACRO-2 asks for, in one dependency
//! line. It works because `#[plexus_macros::activation(client, server_cfg =
//! "feature = \"server\"")]` puts the server half of its expansion behind the
//! same `cfg` the crate puts its server dependencies behind — while the
//! client half, which references only the types below, is emitted
//! unconditionally.
//!
//! The gate: `cargo build -p tickets-api --no-default-features --target
//! wasm32-unknown-unknown`.

pub mod types;
pub use types::{Status, Ticket, TicketsEvent};

// ---------------------------------------------------------------------------
// The server half. Everything in this block is the author's own `#[cfg]` —
// the struct, its state, and the server-only imports. The macro gates *its*
// output with `server_cfg`; it cannot gate the author's.
// ---------------------------------------------------------------------------

#[cfg(feature = "server")]
mod store {
    use super::types::{Status, Ticket};
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    /// Stands in for menger's rusqlite-backed `Store`.
    #[derive(Debug, Clone, Default)]
    pub struct Store(Arc<Mutex<BTreeMap<String, Ticket>>>);

    impl Store {
        pub fn seeded() -> Self {
            let s = Self::default();
            s.insert(Ticket {
                id: "MACRO-1".into(),
                epic: "MACRO".into(),
                title: "the #[activation] macro".into(),
                status: Status::Complete,
                parent: None,
                children: vec!["MACRO-2".into()],
            });
            s.insert(Ticket {
                id: "MACRO-2".into(),
                epic: "MACRO".into(),
                title: "#[activation] generates the typed client".into(),
                status: Status::Ready,
                parent: Some("MACRO-1".into()),
                children: vec![],
            });
            s
        }

        pub fn insert(&self, t: Ticket) {
            self.0.lock().unwrap().insert(t.id.clone(), t);
        }

        pub fn get(&self, id: &str) -> Option<Ticket> {
            self.0.lock().unwrap().get(id).cloned()
        }

        pub fn list(&self, epic: Option<&str>, parent: Option<&str>) -> Vec<Ticket> {
            self.0
                .lock()
                .unwrap()
                .values()
                .filter(|t| epic.is_none_or(|e| t.epic == e))
                .filter(|t| parent.is_none_or(|p| t.parent.as_deref() == Some(p)))
                .cloned()
                .collect()
        }

        pub fn count(&self) -> u64 {
            self.0.lock().unwrap().len() as u64
        }
    }
}

#[cfg(feature = "server")]
pub use store::Store;

/// The activation. Server-only: it holds server state.
#[cfg(feature = "server")]
#[derive(Debug, Clone, Default)]
pub struct Tickets {
    store: Store,
}

#[cfg(feature = "server")]
impl Tickets {
    pub fn new(store: Store) -> Self {
        Self { store }
    }
}

// The attribute itself is NOT `#[cfg]`-gated: it has to run in the
// client-only build too, or there would be no client. `server_cfg` is what
// tells it to put the server half — this impl block included — behind the
// feature.
#[cfg(feature = "server")]
use futures::Stream;

#[plexus_macros::activation(
    namespace = "tickets",
    description = "Plans as data",
    client,
    server_cfg = "feature = \"server\"",
    crate_path = "plexus_core"
)]
impl Tickets {
    /// Heartbeat — version and ticket count.
    #[plexus_macros::method]
    pub async fn health(&self) -> impl Stream<Item = TicketsEvent> + Send + 'static {
        let n = self.store.count();
        futures::stream::iter(vec![TicketsEvent::Health {
            version: env!("CARGO_PKG_VERSION").to_string(),
            ticket_count: n,
        }])
    }

    /// Get one ticket by id.
    #[plexus_macros::method(params(id = "Ticket id, e.g. MACRO-2"))]
    pub async fn get(&self, id: String) -> impl Stream<Item = TicketsEvent> + Send + 'static {
        let ev = match self.store.get(&id) {
            Some(ticket) => TicketsEvent::Ticket { ticket },
            None => TicketsEvent::NotFound { id },
        };
        futures::stream::iter(vec![ev])
    }

    /// List tickets, optionally filtered by epic and/or parent.
    #[plexus_macros::method]
    pub async fn list(
        &self,
        epic: Option<String>,
        parent: Option<String>,
    ) -> impl Stream<Item = TicketsEvent> + Send + 'static {
        let out: Vec<TicketsEvent> = self
            .store
            .list(epic.as_deref(), parent.as_deref())
            .into_iter()
            .map(|ticket| TicketsEvent::Ticket { ticket })
            .collect();
        futures::stream::iter(out)
    }

    /// Subscribe to ticket changes.
    #[plexus_macros::method(streaming)]
    pub async fn watch(
        &self,
        epic: Option<String>,
    ) -> impl Stream<Item = TicketsEvent> + Send + 'static {
        let out: Vec<TicketsEvent> = self
            .store
            .list(epic.as_deref(), None)
            .into_iter()
            .map(|ticket| TicketsEvent::Created { ticket })
            .collect();
        futures::stream::iter(out)
    }
}
