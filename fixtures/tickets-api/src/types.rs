//! The activation's own types. Always compiled — this is the half a wasm32
//! consumer needs, and it depends on nothing but serde/schemars.
//!
//! Shaped after menger's `src/activations/tickets/types.rs` and
//! `src/store.rs`, including the details that bite a hand-mirrored copy:
//! the `#[serde(tag = "type", rename_all = "snake_case")]` event enum, and
//! `skip_serializing_if = "Option::is_none"` on the optional ticket fields.
//!
//! The shared enum is `TicketEvent`, not `TicketsEvent`: on 0.7 the
//! `client` flag derives the activation's aggregate under that name (one
//! variant per method — RETURNS-1 pin 13), so an activation that keeps a
//! hand-written shared enum renames it first. This fixture keeps one to show
//! that a 0.6-shaped activation still compiles with the client on.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TicketEvent {
    Health {
        version: String,
        ticket_count: u64,
    },
    Created {
        ticket: Ticket,
    },
    Ticket {
        ticket: Ticket,
    },
    StatusChanged {
        ticket: Ticket,
        previous: Status,
    },
    NotFound {
        id: String,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Ticket {
    pub id: String,
    pub epic: String,
    pub title: String,
    pub status: Status,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    pub children: Vec<String>,
}

/// No `rename_all` — serializes PascalCase, like menger's `Status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub enum Status {
    Pending,
    Ready,
    Blocked,
    Complete,
}
