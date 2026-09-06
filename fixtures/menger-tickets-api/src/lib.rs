//! MACRO-6 fixture — menger's `tickets` activation, mirrored as a
//! **client-only** activation crate.
//!
//! `tickets-api` (the MACRO-3/4 fixture) is a working activation that also
//! emits a client. This crate is the other shape `#[activation(client)]`
//! supports, and the one a consumer of somebody else's server is in: the
//! method signatures and the event types are declared, the implementation is
//! not. `server_cfg` names a feature that is never enabled, so the server
//! half of the expansion — and the `unreachable!()` bodies below — are never
//! compiled. What *is* compiled is [`types`] and `MengerTicketsClient<T>`.
//!
//! ```text
//!   menger/src/activations/tickets/activation.rs   (the server, untouched)
//!            │  same method names, same params, same event type
//!            ▼
//!   this file  ──#[activation(client)]──►  MengerTicketsClient<T>
//!                                              │
//!                                    WsTransport (MACRO-6)
//!                                              │
//!                                       a live menger
//! ```
//!
//! Why a mirror rather than reusing `tickets-api`: `tickets-api` is a
//! *simplification* of menger (four methods, six events, six ticket fields),
//! so the assertions MACRO-6 has to reproduce — `dangling_blocker`,
//! `unlocks_changed`, `children_changed`, `blocked_by`/`unlocks` — have no
//! types to decode into there, and giving them types would mean
//! reimplementing menger's store in a fixture. Mirroring the declarations
//! costs nothing and keeps the assertions typed.
//!
//! What this fixture is **not**: menger's own client. When MFE-8 lands,
//! menger adds `client` to its own `#[activation]` and the frontend depends
//! on menger's crate — one definition, zero drift. This crate exists so
//! MACRO-6 can prove the transport before that flag is flipped, and it is
//! the reason a drift *would* be caught: the wire test decodes a live
//! server's events into these declarations.

pub mod types;
pub use types::{Kind, Severity, Status, Ticket, TicketsEvent};

/// The activation being mirrored. Never constructed: the server half is
/// behind a `cfg` that is never satisfied.
#[cfg(feature = "server")]
#[derive(Debug, Clone, Default)]
pub struct MengerTickets;

#[plexus_macros::activation(
    namespace = "tickets",
    description = "Plans as data — create, query, and watch the work DAG",
    client,
    server_cfg = "feature = \"server\"",
    crate_path = "plexus_core"
)]
impl MengerTickets {
    /// Heartbeat. Returns the binary version, the server's unix time, and
    /// how many tickets the store holds.
    #[plexus_macros::method(description = "Heartbeat — version, unix time, ticket count")]
    pub async fn health(&self) -> impl Stream<Item = TicketsEvent> + Send + 'static {
        unreachable!("menger-tickets-api is a client-only mirror")
    }

    /// Creates a ticket in an epic. The store allocates the next `EPIC-N`
    /// id; status starts at `Pending`. Yields exactly one event: `Created`
    /// or `Error`.
    #[plexus_macros::method(
        description = "Create a ticket; allocates the next EPIC-N id, status starts Pending",
        params(
            epic = "Epic prefix, e.g. MNG (case-insensitive, stored uppercase)",
            title = "Short description",
            kind = "scope | execution | build | gap | spike (default: build)",
            severity = "Critical | High | Medium | Low (optional)",
            blocked_by = "Ticket ids that must complete first (optional)",
            body = "Markdown body (optional)",
            parent = "Id of the ticket this one is an output of; any epic, must exist (optional)"
        )
    )]
    pub async fn create(
        &self,
        epic: String,
        title: String,
        kind: Option<String>,
        severity: Option<String>,
        blocked_by: Option<Vec<String>>,
        body: Option<String>,
        parent: Option<String>,
    ) -> impl Stream<Item = TicketsEvent> + Send + 'static {
        unreachable!("menger-tickets-api is a client-only mirror")
    }

    /// Yields exactly one event: `Ticket` if found, `NotFound` otherwise.
    #[plexus_macros::method(
        description = "Get one ticket by id",
        params(id = "Ticket id, e.g. MNG-2")
    )]
    pub async fn get(&self, id: String) -> impl Stream<Item = TicketsEvent> + Send + 'static {
        unreachable!("menger-tickets-api is a client-only mirror")
    }

    /// Yields one `Ticket` event per match, ordered by (epic, seq).
    #[plexus_macros::method(
        description = "List tickets, optionally filtered by epic, status and/or parent",
        params(
            epic = "Epic prefix filter (optional)",
            status = "Status filter (optional)",
            parent = "Only this ticket's children (optional)"
        )
    )]
    pub async fn list(
        &self,
        epic: Option<String>,
        status: Option<String>,
        parent: Option<String>,
    ) -> impl Stream<Item = TicketsEvent> + Send + 'static {
        unreachable!("menger-tickets-api is a client-only mirror")
    }

    /// Yields exactly one event: `StatusChanged`, `NotFound`, or `Error`.
    #[plexus_macros::method(
        description = "Set a ticket's status",
        params(id = "Ticket id, e.g. MNG-2", status = "New status")
    )]
    pub async fn set_status(
        &self,
        id: String,
        status: String,
    ) -> impl Stream<Item = TicketsEvent> + Send + 'static {
        unreachable!("menger-tickets-api is a client-only mirror")
    }

    /// The DAG query: one `Ticket` event per member of the ready set.
    #[plexus_macros::method(
        description = "Tickets that can start now: Ready, with every blocker Complete or Superseded",
        params(epic = "Only tickets in this epic (optional)")
    )]
    pub async fn ready_set(
        &self,
        epic: Option<String>,
    ) -> impl Stream<Item = TicketsEvent> + Send + 'static {
        unreachable!("menger-tickets-api is a client-only mirror")
    }

    /// "Why can't this start?": one `Blocker` per resolved blocker and one
    /// `DanglingBlocker` per id that names no ticket.
    #[plexus_macros::method(
        description = "One ticket's blockers resolved to current statuses; dangling ids marked",
        params(id = "Ticket id whose blockers to resolve")
    )]
    pub async fn blockers(
        &self,
        id: String,
    ) -> impl Stream<Item = TicketsEvent> + Send + 'static {
        unreachable!("menger-tickets-api is a client-only mirror")
    }

    /// The reactive primitive: every mutation, until the client
    /// disconnects.
    #[plexus_macros::method(
        streaming,
        description = "Subscribe to ticket changes, until disconnect",
        params(epic = "Only changes in this epic (optional)")
    )]
    pub async fn watch(
        &self,
        epic: Option<String>,
    ) -> impl Stream<Item = TicketsEvent> + Send + 'static {
        unreachable!("menger-tickets-api is a client-only mirror")
    }
}
