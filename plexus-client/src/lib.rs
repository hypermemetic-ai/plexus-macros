//! # plexus-client
//!
//! The **wasm-safe client runtime** for Plexus RPC: the wire envelope a
//! client receives, the `Transport` seam every wire implementation
//! satisfies, and the decoder that turns wire items into an activation's own
//! typed events.
//!
//! This crate is the runtime half of `#[plexus_macros::activation(client)]`.
//! The macro emits the per-activation typed client; everything the emitted
//! code leans on lives here, so the generated code stays small and every
//! transport is written once for all activations.
//!
//! ## Why it is a separate crate
//!
//! `plexus-core` is the server: tokio (full), jsonrpsee, rmcp, http. None of
//! that builds for `wasm32-unknown-unknown`, and gating ~9k lines of
//! `plexus.rs`/`schema.rs` behind features to make it would be a far larger
//! and riskier change than extracting the ~400 lines a *client* actually
//! needs. So the client contract lives here, with four dependencies —
//! `serde`, `serde_json`, `futures-core`, `futures-util` — all of which
//! build for wasm32.
//!
//! `cargo build -p plexus-client --target wasm32-unknown-unknown` is the
//! regression gate on that claim.
//!
//! ## The shape
//!
//! ```text
//!   generated client method
//!         │  builds the by-name params object from typed arguments
//!         ▼
//!   Transport::call(MethodRef, Value) ──► ItemStream    (wire items)
//!         │
//!         ▼
//!   decode::<E>(..)                   ──► EventStream<E> (typed events)
//! ```
//!
//! Two failure layers stay apart the whole way down: a
//! [`TransportError`] is the connection failing, a
//! [`ClientError::Remote`] is the server answering with an in-band `error`
//! item. See [`error`].
//!
//! ## Transports
//!
//! [`MemoryTransport`] ships here because it is the only transport with no
//! I/O and therefore the only one that is wasm-safe by construction; it is
//! the test double. The in-process transport lives on the `plexus-core`
//! side, because it drives a real `Activation`.
//!
//! [`WsTransport`] — WebSocket JSON-RPC, the transport that carries the full
//! envelope — ships here too, but **behind a feature**, because a socket is
//! I/O and the four-dependency core must stay the default:
//!
//! | Feature | Backend | Target |
//! |---|---|---|
//! | *(none)* | — | the wasm-clean core: `serde`, `serde_json`, `futures-core`, `futures-util` |
//! | `ws-native` | `tokio-tungstenite` | native |
//! | `ws-web` | `web_sys::WebSocket` | `wasm32-unknown-unknown` |
//!
//! ## Many streams as one
//!
//! [`watch`] merges a tuple of method streams into one whose item is exactly
//! the union of theirs — `watch((tickets, chats, deployments))` yields
//! `Watched3<A, B, C>`, and dropping it drops (on a `WsTransport`:
//! unsubscribes) each (RETURNS-38). See [`watch`](mod@watch).
//!
//! Both features expose the same `WsTransport` type and the same
//! `WsTransport::connect(url, hub_namespace)`, so a consumer picks the
//! backend in `Cargo.toml` and changes no code. See [`ws`].

#![forbid(unsafe_code)]
#![deny(missing_debug_implementations)]

pub mod contract;
pub mod decode;
pub mod envelope;
pub mod error;
pub mod event;
pub mod memory;
pub mod transport;
pub mod watch;

#[cfg(any(feature = "ws-native", feature = "ws-web"))]
pub mod ws;

pub use contract::{error_events, into_events, owned_call, BoxStream, FromClientError};
pub use decode::{collect_data, decode, error_stream, EventStream};
pub use envelope::{StreamItem, StreamMetadata};
pub use error::{ClientError, TransportError, TransportErrorKind};
pub use event::ClientEvent;
pub use memory::MemoryTransport;
pub use transport::{ItemStream, MethodRef, Transport};
pub use watch::{
    watch, watch_with, Errors, Fallible, StopOnError, Watch, Watched10, Watched11, Watched12, Watched2, Watched3,
    Watched4, Watched5, Watched6, Watched7, Watched8, Watched9, Watching10, Watching11, Watching12, Watching2,
    Watching3, Watching4, Watching5, Watching6, Watching7, Watching8, Watching9,
};

#[cfg(any(feature = "ws-native", feature = "ws-web"))]
pub use ws::WsTransport;

/// Re-exported so generated client code never needs `serde_json` in the
/// consuming crate's own `Cargo.toml`.
#[doc(hidden)]
pub mod __rt {
    pub use serde_json;
    pub use crate::transport::join_path;
}
