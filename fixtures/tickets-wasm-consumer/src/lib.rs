//! MACRO-2 criterion 1, as code a browser could run.
//!
//! This crate is what menger's Leptos frontend becomes under MFE-8: it holds
//! **no** wire client and **no** mirrored ticket type. It depends on
//! `tickets-api` with `default-features = false` and calls the generated
//! `TicketsClient`. The whole file compiles for `wasm32-unknown-unknown`.
//!
//! Compare `menger/frontend/src/net.rs` (145 lines of `fetch` +
//! `ReadableStream` + hand-rolled SSE framing) and `frontend/src/main.rs`
//! lines 102–141 (a `Ticket` struct with every enum flattened to `String`
//! and every field `#[serde(default)]`, plus a third hand-copy of the status
//! vocabulary). Both disappear.

use futures::StreamExt;
use plexus_client::{ClientError, ClientEvent, EventStream, Transport};

// The types are the server's own — not a copy. That identity is what makes
// a field rename on the server a compile error here.
pub use tickets_api::{Status, Ticket, TicketsClient, TicketsEvent};

/// Every ticket in an epic, as typed values.
///
/// The unary shape: drive the stream to completion, collect the payloads.
pub async fn tickets_in_epic<T: Transport>(
    client: &TicketsClient<T>,
    epic: &str,
) -> Result<Vec<Ticket>, ClientError> {
    let events = client.list_all(Some(epic.to_string()), None).await?;
    Ok(events
        .into_iter()
        .filter_map(|e| match e {
            TicketsEvent::Ticket { ticket } => Some(ticket),
            _ => None,
        })
        .collect())
}

/// The `watch` shape: fold a live stream into a local mirror.
///
/// The `match` below is the reason this is worth doing. It is exhaustive
/// over `TicketsEvent`, so a new server-side variant is a compile error
/// here — not a silently ignored event, which is what
/// `match ev["type"].as_str()` gives you.
pub async fn mirror_epic<T: Transport>(
    client: &TicketsClient<T>,
    epic: Option<String>,
) -> Result<Vec<Ticket>, ClientError> {
    let mut mirror: Vec<Ticket> = Vec::new();
    let mut stream: EventStream<'_, TicketsEvent> = client.watch(epic);

    while let Some(event) = stream.next().await {
        match event? {
            ClientEvent::Data(TicketsEvent::Created { ticket })
            | ClientEvent::Data(TicketsEvent::Ticket { ticket })
            | ClientEvent::Data(TicketsEvent::StatusChanged { ticket, .. }) => {
                match mirror.iter_mut().find(|t| t.id == ticket.id) {
                    Some(slot) => *slot = ticket,
                    None => mirror.push(ticket),
                }
            }
            ClientEvent::Data(TicketsEvent::Health { .. })
            | ClientEvent::Data(TicketsEvent::NotFound { .. }) => {}
            ClientEvent::Data(TicketsEvent::Error { message }) => {
                // An activation-level error variant. Distinct from
                // `Err(ClientError::Remote)` — this one is a value the
                // activation chose to emit, not a failed call.
                return Err(ClientError::Remote {
                    message,
                    code: None,
                    recoverable: true,
                });
            }
            ClientEvent::Progress { .. } => {}
        }
    }
    Ok(mirror)
}

/// A status transition, rendered for a UI. `Status` is the server's enum, so
/// the vocabulary cannot drift — menger currently keeps three hand-copies of
/// it (server `Status`, `web/index.html`, `frontend/src/main.rs`).
pub fn describe(status: Status) -> &'static str {
    match status {
        Status::Pending => "awaiting ratification",
        Status::Ready => "eligible to start",
        Status::Blocked => "waiting on a blocker",
        Status::Complete => "done",
    }
}
