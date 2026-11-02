//! The Plexus wire envelope, as a **client** sees it.
//!
//! # Why this is not `plexus_core::plexus::types::PlexusStreamItem`
//!
//! The server-side envelope requires `metadata` on every non-`Request`
//! variant, and the WebSocket JSON-RPC transport does deliver it. The REST
//! bridge does **not**: `stream_sse_response` re-frames each item and emits
//! only
//!
//! ```text
//! event: data
//! data: {"content": <event JSON>}
//! ```
//!
//! — dropping `metadata` and `content_type` entirely (likewise
//! `event: error` carries only `{"message", "code"}`, with no `recoverable`).
//! A client type that made those fields mandatory could therefore not
//! represent what the REST transport actually receives.
//!
//! So [`StreamItem`] is deliberately the **looser** type: every field the
//! REST bridge drops is `Option` / `#[serde(default)]`. It deserializes the
//! full WebSocket envelope losslessly, *and* an SSE transport can construct
//! the same type from an `(event-name, data-payload)` pair — which is what
//! makes one typed client work over both transports.
//!
//! (An SSE `data:` payload is not itself a `StreamItem`: the discriminant
//! lives in the frame's `event:` field, not in the JSON. Reassembling the
//! two is the SSE transport's job; this type is the target shape it
//! reassembles *into*.)
//!
//! Wire compatibility with the server envelope is a regression gate, not an
//! assumption — see `tests/client_envelope_compat.rs` in `plexus-macros`,
//! which round-trips every `PlexusStreamItem` variant produced by
//! `plexus-core` through this type.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use serde_json::Value;
use std::sync::Arc;

/// Provenance metadata the calling layer stamps onto each item.
///
/// Present on the WebSocket JSON-RPC transport; absent over REST + SSE.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct StreamMetadata {
    /// Call path through the system (e.g. `["menger", "tickets"]`).
    #[serde(default)]
    pub provenance: Vec<String>,
    /// Hash of the server's Plexus configuration, for cache invalidation.
    #[serde(default)]
    pub plexus_hash: String,
    /// Unix timestamp (seconds) at which the item was wrapped.
    #[serde(default)]
    pub timestamp: i64,
}

/// One item of a Plexus response stream, as delivered by a [`Transport`].
///
/// [`Transport`]: crate::Transport
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamItem {
    /// A payload item. `content` is the activation's own event, still as
    /// JSON; [`crate::decode`] is what turns it into the method's event type.
    Data {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        metadata: Option<StreamMetadata>,
        /// e.g. `"tickets.watch"`. Absent over REST + SSE.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        content_type: Option<String>,
        content: Value,
    },

    /// A progress report. Carries no payload; never terminates the stream.
    Progress {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        metadata: Option<StreamMetadata>,
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        percentage: Option<f32>,
    },

    /// An **in-band** error raised by the hub or the activation — as opposed
    /// to a [`crate::TransportError`], which is the connection failing.
    /// Terminates the stream.
    Error {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        metadata: Option<StreamMetadata>,
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        code: Option<String>,
        /// The REST bridge does not emit this field; it defaults to `false`.
        #[serde(default)]
        recoverable: bool,
    },

    /// A server-to-client request (bidirectional activations). The typed
    /// client has no answer channel, so [`crate::decode`] skips these; a
    /// consumer that needs them reads the raw [`StreamItem`] stream.
    #[serde(rename_all = "camelCase")]
    Request {
        request_id: String,
        request_data: Value,
        #[serde(default)]
        timeout_ms: u64,
    },

    /// Normal end of stream.
    Done {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        metadata: Option<StreamMetadata>,
    },
}

impl StreamItem {
    /// Convenience constructor for a data item with no server metadata —
    /// the shape the REST + SSE bridge produces.
    pub fn data(content: Value) -> Self {
        StreamItem::Data {
            metadata: None,
            content_type: None,
            content,
        }
    }

    /// Convenience constructor for a terminal `done` item.
    pub fn done() -> Self {
        StreamItem::Done { metadata: None }
    }

    /// `true` for the two variants after which no further item may arrive.
    pub fn is_terminal(&self) -> bool {
        matches!(self, StreamItem::Done { .. } | StreamItem::Error { .. })
    }

    /// The provenance metadata, when the transport carried it.
    pub fn metadata(&self) -> Option<&StreamMetadata> {
        match self {
            StreamItem::Data { metadata, .. }
            | StreamItem::Progress { metadata, .. }
            | StreamItem::Error { metadata, .. }
            | StreamItem::Done { metadata } => metadata.as_ref(),
            StreamItem::Request { .. } => None,
        }
    }
}

/// An item's content as a typed client reads it (PERF.115-36): the JSON text
/// the wire carried, decoded straight into the method's event when it is
/// read, never into a [`Value`] first. A transport that has no text (one in
/// process, a script) hands over the `Value` it holds.
#[derive(Debug, Clone)]
pub struct Content(Repr);

#[derive(Debug, Clone)]
enum Repr {
    Text(Box<RawValue>),
    Value(Value),
}

impl Content {
    /// The JSON text the wire carried, when the transport kept it.
    pub fn as_text(&self) -> Option<&str> {
        match &self.0 {
            Repr::Text(text) => Some(text.get()),
            Repr::Value(_) => None,
        }
    }

    /// The content decoded into `E`: from the text, or from the `Value`.
    pub fn decode<E: DeserializeOwned>(self) -> Result<E, serde_json::Error> {
        match self.0 {
            Repr::Text(text) => serde_json::from_str(text.get()),
            Repr::Value(value) => serde_json::from_value(value),
        }
    }

    /// The content as a `Value`, for a caller that wants one.
    pub fn into_value(self) -> Result<Value, serde_json::Error> {
        match self.0 {
            Repr::Text(text) => serde_json::from_str(text.get()),
            Repr::Value(value) => Ok(value),
        }
    }
}

impl From<Box<RawValue>> for Content {
    fn from(text: Box<RawValue>) -> Self {
        Content(Repr::Text(text))
    }
}

impl From<Value> for Content {
    fn from(value: Value) -> Self {
        Content(Repr::Value(value))
    }
}

/// One item of a response stream as a typed client reads it (PERF.115-36):
/// a [`StreamItem`] whose content is still the wire's JSON ([`Content`]), and
/// nothing read that a typed decode does not use: no metadata, no request's
/// data. [`Transport::call_raw`](crate::Transport::call_raw) yields these and
/// [`decode_raw`](crate::decode_raw) turns them into events; a consumer that
/// wants the metadata, or answers requests, reads [`StreamItem`]s.
#[derive(Debug, Clone)]
pub enum RawItem {
    /// A payload item: its content, and its `content_type`, which every item
    /// of a call shares.
    Data {
        content_type: Option<Arc<str>>,
        content: Content,
    },
    /// A progress report. Never terminates the stream.
    Progress { message: String, percentage: Option<f32> },
    /// An in-band error raised by the service. Terminates the stream.
    Error {
        message: String,
        code: Option<String>,
        recoverable: bool,
    },
    /// A server-to-client request. A typed client has no answer channel and
    /// skips it.
    Request,
    /// Normal end of stream.
    Done,
}

impl RawItem {
    /// `true` for the two variants after which no further item may arrive.
    pub fn is_terminal(&self) -> bool {
        matches!(self, RawItem::Done | RawItem::Error { .. })
    }
}

impl From<StreamItem> for RawItem {
    fn from(item: StreamItem) -> Self {
        match item {
            StreamItem::Data { content_type, content, .. } => RawItem::Data {
                content_type: content_type.map(Arc::from),
                content: content.into(),
            },
            StreamItem::Progress { message, percentage, .. } => RawItem::Progress { message, percentage },
            StreamItem::Error { message, code, recoverable, .. } => RawItem::Error { message, code, recoverable },
            StreamItem::Request { .. } => RawItem::Request,
            StreamItem::Done { .. } => RawItem::Done,
        }
    }
}
