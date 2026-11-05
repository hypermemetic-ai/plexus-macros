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

pub mod contract;
pub mod decode;
pub mod envelope;
pub mod error;
pub mod frame;
pub mod lift;
pub mod memory;
pub mod method_stream;
pub mod params;
pub mod progress;
pub mod remote_error;
pub mod route;
/// RETURNS-36: a peer on plexus-wire's spec as main wrote it, read as 0.8.
pub mod spec05;
pub mod transport;
/// PERF.115-43: `#[derive(plexus_macros::Deserialize)]`'s runtime, reached
/// through `__rt::type_first`.
mod type_first;
pub mod watch;
pub mod wire_result;

#[cfg(any(feature = "ws-native", feature = "ws-web"))]
pub mod ws;

pub use contract::{BoxFuture, BoxStream, FromClientError, MaybeSend};
pub use decode::{decode, decode_raw, decode_result, decode_with_progress};
pub use envelope::{Content, RawItem, StreamItem, StreamMetadata};
pub use error::{ClientError, NameRefusal, TransportError, TransportErrorKind};
pub use frame::Frame;
pub use lift::{Aggregated, HasClient, Lift, Named, Root, Static, Wrap};
pub use route::Route;
pub use memory::MemoryTransport;
pub use method_stream::{call_one, call_one_with_input, Decoder, EventOf, Events, MethodStream, NoProgress, WithProgress};
pub use params::Params;
pub use progress::{Note, Progress};
pub use remote_error::{DenyReason, OtherError, ParamProblem, RemoteError};
pub use transport::{encode_input, InputItem, InputItems, ItemStream, MethodRef, RawItemStream, RoutePath, Transport};
pub use watch::{
    watch, watch_with, Errors, Fallible, StopOnError, Watch, Watched10, Watched11, Watched12, Watched2, Watched3,
    Watched4, Watched5, Watched6, Watched7, Watched8, Watched9, Watching10, Watching11, Watching12, Watching2,
    Watching3, Watching4, Watching5, Watching6, Watching7, Watching8, Watching9,
};

#[cfg(any(feature = "ws-native", feature = "ws-web"))]
pub use ws::WsTransport;

/// Re-exported so generated client code never needs `serde` or `serde_json`
/// in the consuming crate's own `Cargo.toml`: the `{Ns}Event` aggregate
/// derives through `__rt::serde` (`#[serde(crate = "…")]`).
#[doc(hidden)]
pub mod __rt {
    pub use serde;
    pub use serde_json;
    pub use futures_core::Stream;

    /// PERF.115-43: what `#[derive(plexus_macros::Deserialize)]` emits calls:
    /// an internally tagged enum read with its tag first, serde's buffered
    /// read where it is not (see the module's source, `type_first.rs`).
    pub mod type_first {
        pub use crate::type_first::{deserialize, TypeFirst};
    }
}
