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
//!
//! On 0.7 (RETURNS-9) a one-item method is an `async fn` returning its
//! item, a many-item method a `MethodStream` (a plain `TryStream`, with
//! `.with_progress()` and `.events()`), and a failed call a typed
//! `ClientError::Remote(RemoteError)`.

use futures::{StreamExt, TryStreamExt};
use plexus_client::{ClientError, Transport};

// The types are the server's own — not a copy. That identity is what makes
// a field rename on the server a compile error here. `TicketsEvent` is the
// activation's aggregate, derived by the macro beside `TicketsClient`.
pub use tickets_api::{Status, Ticket, TicketEvent, TicketsClient, TicketsEvent};

/// Every ticket in an epic, as typed values.
///
/// A many-item method is a plain `TryStream`: collect it.
pub async fn tickets_in_epic<T: Transport>(
    client: &TicketsClient<T>,
    epic: &str,
) -> Result<Vec<Ticket>, ClientError> {
    let events: Vec<TicketEvent> = client
        .list(Some(epic.to_string()), None)
        .try_collect()
        .await?;
    Ok(events
        .into_iter()
        .filter_map(|e| match e {
            TicketEvent::Ticket { ticket } => Some(ticket),
            _ => None,
        })
        .collect())
}

/// The `watch` shape: fold a live stream into a local mirror.
///
/// Two layers, nested (RETURNS-1 decision 8): the outer `Result` is the call
/// failing (`?`), the inner one the activation saying no — here its own
/// `Error` variant, a value it chose to emit.
///
/// The `match` below is the reason this is worth doing. It is exhaustive
/// over `TicketEvent`, so a new server-side variant is a compile error
/// here — not a silently ignored event, which is what
/// `match ev["type"].as_str()` gives you.
pub async fn mirror_epic<T: Transport>(
    client: &TicketsClient<T>,
    epic: Option<String>,
) -> Result<Result<Vec<Ticket>, String>, ClientError> {
    let mut mirror: Vec<Ticket> = Vec::new();
    let mut stream = client.watch(epic);

    while let Some(event) = stream.next().await {
        match event? {
            TicketEvent::Created { ticket }
            | TicketEvent::Ticket { ticket }
            | TicketEvent::StatusChanged { ticket, .. } => upsert(&mut mirror, ticket),
            TicketEvent::Health { .. } | TicketEvent::NotFound { .. } => {}
            TicketEvent::Error { message } => return Ok(Err(message)),
        }
    }
    Ok(Ok(mirror))
}

/// One listener over several calls: `list` and `watch` merged through the
/// activation's aggregate (`.events()`), so each item says which call it
/// came from — ordinary stream plumbing, nothing built in (RETURNS-1
/// decision 6).
pub async fn listed_and_watched<T: Transport>(
    client: &TicketsClient<T>,
    epic: &str,
) -> Result<Vec<Ticket>, ClientError> {
    let mut all = futures::stream::select_all([
        client.list(Some(epic.to_string()), None).events(),
        client.watch(Some(epic.to_string())).events(),
    ]);
    let mut mirror: Vec<Ticket> = Vec::new();
    while let Some(event) = all.next().await {
        match event? {
            TicketsEvent::List(TicketEvent::Ticket { ticket })
            | TicketsEvent::Watch(
                TicketEvent::Created { ticket } | TicketEvent::StatusChanged { ticket, .. },
            ) => upsert(&mut mirror, ticket),
            TicketsEvent::List(_) | TicketsEvent::Watch(_) => {}
            // One-item methods have no stream to merge; their variants
            // exist for a receiver that wraps their results itself.
            TicketsEvent::Health(_) | TicketsEvent::Get(_) => {}
        }
    }
    Ok(mirror)
}

fn upsert(mirror: &mut Vec<Ticket>, ticket: Ticket) {
    match mirror.iter_mut().find(|t| t.id == ticket.id) {
        Some(slot) => *slot = ticket,
        None => mirror.push(ticket),
    }
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
