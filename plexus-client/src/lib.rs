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
//!         │  builds the by-name params object from typed arguments (Params)
//!         ▼
//!   Transport::call(MethodRef, Value) ──► ItemStream          (wire items)
//!         │
//!         ├─ one-item method  ── call_one ──► Result<I, ClientError>
//!         └─ many-item method ──────────────► MethodStream<'_, I, P, {Ns}Event>
//!                                               ├─ Stream<Item = Result<I, ClientError>>   (progress dropped)
//!                                               ├─ .with_progress() → Result<Frame<I, P>, ClientError>
//!                                               └─ .events()        → Result<{Ns}Event, ClientError>
//! ```
//!
//! `I` is the method's item — a plain `T`, or `Result<T, E>` decoded from its
//! `{"ok"}` / `{"err"}` wire form — `P` its progress type ([`NoProgress`] when
//! it declares none) and `{Ns}Event` the activation's aggregate, which the
//! macro derives beside the client.
//!
//! Two layers stay apart the whole way down (RETURNS-1 decision 8): a
//! [`ClientError`] is the call failing — the connection
//! ([`ClientError::Transport`]) or the server's typed error frame
//! ([`ClientError::Remote`]`(`[`RemoteError`]`)`) — and an `err` item is the
//! activation saying no, which arrives as a value. `?` is the call failing;
//! a match on the item is the activation's answer. See [`error`].
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
//! Both features expose the same `WsTransport` type and the same
//! `WsTransport::connect(url, hub_namespace)`, so a consumer picks the
//! backend in `Cargo.toml` and changes no code. See [`ws`].
//!
//! ## The 0.7 vocabulary
//!
//! The types every party to a precise-returns call names — the server
//! author, the macro, the generated client, the generators' mirrors — live
//! here once, because this is the one crate all of them link:
//!
//! | Type | What it is | On the wire |
//! |---|---|---|
//! | [`Progress<T, P = Note>`](Progress) | a method's item when it reports progress: `Update(P)` \| `Item(T)` | never as a whole: `Update` → progress frame, `Item` → data frame |
//! | [`Note`] | the default progress type | `{"message": …, "percentage": …}` |
//! | [`Frame<I, P>`](Frame) | what an opted-in client stream yields: `Item(I)` \| `Progress(P)` | client-side only |
//! | [`RemoteError`] (+ [`ParamProblem`], [`DenyReason`]) | why a call failed — typed, exhaustive | `kind`-tagged object |
//! | [`wire_result`] | serde `with` module for a Result item | `{"ok": t}` \| `{"err": e}` |
//!
//! `Note` and `RemoteError` implement `schemars::JsonSchema` under the
//! `schema` feature (off by default: a wasm32 consumer that only decodes
//! needs none of it).

#![forbid(unsafe_code)]
#![deny(missing_debug_implementations)]

pub mod decode;
pub mod envelope;
pub mod error;
pub mod frame;
pub mod memory;
pub mod method_stream;
pub mod params;
pub mod progress;
pub mod remote_error;
pub mod transport;
pub mod wire_result;

#[cfg(any(feature = "ws-native", feature = "ws-web"))]
pub mod ws;

pub use decode::{decode, decode_result, decode_with_progress};
pub use envelope::{StreamItem, StreamMetadata};
pub use error::{ClientError, TransportError, TransportErrorKind};
pub use frame::Frame;
pub use memory::MemoryTransport;
pub use method_stream::{call_one, Decoder, Events, MethodStream, NoProgress, WithProgress};
pub use params::Params;
pub use progress::{Note, Progress};
pub use remote_error::{DenyReason, ParamProblem, RemoteError};
pub use transport::{ItemStream, MethodRef, Transport};

#[cfg(any(feature = "ws-native", feature = "ws-web"))]
pub use ws::WsTransport;

/// Re-exported so generated client code never needs `serde` or `serde_json`
/// in the consuming crate's own `Cargo.toml`: the `{Ns}Event` aggregate
/// derives through `__rt::serde` (`#[serde(crate = "…")]`).
#[doc(hidden)]
pub mod __rt {
    pub use serde;
    pub use serde_json;
}
