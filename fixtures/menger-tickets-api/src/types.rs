//! menger's ticket vocabulary, mirrored field for field.
//!
//! Source of truth: `menger/src/store.rs` (`Ticket`, `Status`, `Kind`,
//! `Severity`) and `menger/src/activations/tickets/types.rs`
//! (`TicketsEvent`), read at 2026-09-06. This file is a *declaration* of
//! what the live server sends — not a hand-written parser of it, and not a
//! loosened copy: no `#[serde(default)]`, no `Value` escape hatches. If
//! menger renames a field, the decode fails loudly here and MFE-8's real
//! dependency (menger's own crate with the `client` flag) would fail to
//! compile — which is the property MACRO-2 exists to buy.

use serde::{Deserialize, Serialize};

/// Every event menger's `tickets` activation can emit.
///
/// `#[serde(tag = "type", rename_all = "snake_case")]` is what puts
/// `{"type":"created", …}` on the wire, and it is why an unknown variant is
/// a hard decode error rather than a silently ignored event — the shape
/// `match ev["type"].as_str()` in `web/index.html` cannot give you.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TicketsEvent {
    /// Heartbeat: binary version, server unix time, ticket count.
    Health {
        version: String,
        unix_now: i64,
        ticket_count: u64,
    },
    /// A ticket was created (from `create`, and from `watch` on every birth).
    Created { ticket: Ticket },
    /// One ticket as it exists now (from `get`, and per row from `list` and
    /// `ready_set`).
    Ticket { ticket: Ticket },
    /// A status transition (from `set_status`, and from `watch`).
    StatusChanged { ticket: Ticket, previous: Status },
    /// An existing ticket's `unlocks` gained `added`, because a newly
    /// created ticket named it in `blocked_by`. Carries the full updated
    /// ticket, so a watch-only mirror stays coherent with one upsert.
    UnlocksChanged { ticket: Ticket, added: String },
    /// An existing ticket's `children` gained `added` — a child was born
    /// naming it as `parent`. Same discipline: the full parent rides along.
    ChildrenChanged { ticket: Ticket, added: String },
    /// One resolved blocker of the queried ticket, at its current status.
    Blocker { ticket: Ticket },
    /// A `blocked_by` id that names no ticket — a dangling forward
    /// reference. A distinct event category, not prose in a message.
    DanglingBlocker { id: String },
    /// Lookup miss.
    NotFound { id: String },
    /// In-band error (bad enum value, invalid epic, `blocked_by` cycle, a
    /// parent that does not exist, …). A *data* event: the call succeeded
    /// and the activation said no, which is not the same thing as
    /// [`ClientError::Remote`](plexus_client::ClientError::Remote).
    Error { message: String },
}

/// menger's `Ticket`, as it appears on the wire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Ticket {
    /// `EPIC-N`, allocated by the store.
    pub id: String,
    /// Epic prefix, uppercase.
    pub epic: String,
    /// Sequence number within the epic (the `N` in `EPIC-N`).
    pub seq: u32,
    pub title: String,
    pub status: Status,
    pub kind: Kind,
    /// Absent, not `null`, when unset — menger uses
    /// `skip_serializing_if = "Option::is_none"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub severity: Option<Severity>,
    /// Ticket ids that must complete before this one starts.
    pub blocked_by: Vec<String>,
    /// Ticket ids that can start once this one completes — derived by the
    /// store from other tickets' `blocked_by`.
    pub unlocks: Vec<String>,
    /// The ticket this one is an output of, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// The tickets whose `parent` is this one, in birth order.
    pub children: Vec<String>,
    /// Markdown body.
    pub body: String,
    pub created_unix: i64,
    pub updated_unix: i64,
}

/// Lifecycle status. Frontmatter casing on the wire (`"Pending"`), not
/// `snake_case` — menger's `Status` carries no `rename_all`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub enum Status {
    Pending,
    Ready,
    Blocked,
    Complete,
    Idea,
    Epic,
    Superseded,
}

/// What shape of work a ticket is — the owner's closed five. Serialized
/// lowercase (`"build"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Scope,
    Execution,
    Build,
    Gap,
    Spike,
}

/// PascalCase on the wire, like `Status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub enum Severity {
    Critical,
    High,
    Medium,
    Low,
}
