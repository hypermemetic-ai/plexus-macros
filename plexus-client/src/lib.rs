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
//! the test double. The real transports live where their dependencies do:
//! in-process on the `plexus-core` side (it drives a real `Activation`),
//! WebSocket JSON-RPC and REST + SSE in their own crates.

#![forbid(unsafe_code)]
#![deny(missing_debug_implementations)]

pub mod decode;
pub mod envelope;
pub mod error;
pub mod event;
pub mod memory;
pub mod transport;

pub use decode::{collect_data, decode, error_stream, EventStream};
pub use envelope::{StreamItem, StreamMetadata};
pub use error::{ClientError, TransportError, TransportErrorKind};
pub use event::ClientEvent;
pub use memory::MemoryTransport;
pub use transport::{ItemStream, MethodRef, Transport};

/// Re-exported so generated client code never needs `serde_json` in the
/// consuming crate's own `Cargo.toml`.
#[doc(hidden)]
pub mod __rt {
    pub use serde_json;
}
