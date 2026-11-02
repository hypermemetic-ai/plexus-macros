//! Turning a wire-item stream into a typed event stream.
//!
//! This is the one piece of runtime the generated client calls: everything
//! above it is the activation's own types, everything below it is a
//! [`Transport`](crate::Transport).

use crate::envelope::{RawItem, StreamItem};
use crate::error::ClientError;
use crate::event::ClientEvent;
use crate::transport::{ItemStream, RawItemStream};
use futures_core::Stream;
use futures_util::{stream, StreamExt};
use serde::de::DeserializeOwned;
use std::pin::Pin;

/// A boxed stream of typed events. Mirrors [`ItemStream`]'s `Send` bound:
/// required off wasm32, dropped on it.
#[cfg(not(target_arch = "wasm32"))]
pub type EventStream<'a, E> =
    Pin<Box<dyn Stream<Item = Result<ClientEvent<E>, ClientError>> + Send + 'a>>;

/// A boxed stream of typed events. See the non-wasm32 variant.
#[cfg(target_arch = "wasm32")]
pub type EventStream<'a, E> = Pin<Box<dyn Stream<Item = Result<ClientEvent<E>, ClientError>> + 'a>>;

/// Decode a transport's wire items into a method's typed events.
///
/// The mapping, exhaustively:
///
/// | Wire item | Yielded | Stream continues? |
/// |---|---|---|
/// | `data` (deserializes) | `Ok(ClientEvent::Data(e))` | yes |
/// | `data` (does not) | `Err(ClientError::Decode)` | **no** |
/// | `progress` | `Ok(ClientEvent::Progress)` | yes |
/// | `error` | `Err(ClientError::Remote)` | **no** |
/// | `request` | *skipped* | yes |
/// | `done` | *nothing* | **no** |
/// | transport failure | `Err(ClientError::Transport)` | **no** |
///
/// Both terminal errors end the stream because both mean the server will
/// send nothing further that is meaningful: an in-band `error` is the hub's
/// last word on the call (`stream_sse_response` and the buffered REST path
/// both stop there), and a decode failure means client and server disagree
/// about the event type, so every subsequent item would fail the same way.
///
/// A `done` item is *not* surfaced as an event — normal completion is the
/// stream ending. `Ok(())`-style sentinels in a `Stream` only invite
/// consumers to forget to handle them.
pub fn decode<'a, E>(items: ItemStream<'a>) -> EventStream<'a, E>
where
    E: DeserializeOwned + 'a,
{
    Box::pin(stream::unfold(Some(items), |state| async move {
        let mut items = state?;
        loop {
            let next = items.next().await?;
            match next {
                Err(e) => return Some((Err(ClientError::Transport(e)), None)),
                Ok(StreamItem::Done { .. }) => return None,
                Ok(StreamItem::Data {
                    content,
                    content_type,
                    ..
                }) => {
                    return Some(match serde_json::from_value::<E>(content) {
                        Ok(event) => (Ok(ClientEvent::Data(event)), Some(items)),
                        Err(e) => (
                            Err(ClientError::Decode {
                                content_type,
                                message: e.to_string(),
                            }),
                            None,
                        ),
                    })
                }
                Ok(StreamItem::Progress {
                    message,
                    percentage,
                    ..
                }) => {
                    return Some((
                        Ok(ClientEvent::Progress {
                            message,
                            percentage,
                        }),
                        Some(items),
                    ))
                }
                Ok(StreamItem::Error {
                    message,
                    code,
                    recoverable,
                    ..
                }) => {
                    return Some((
                        Err(ClientError::Remote {
                            message,
                            code,
                            recoverable,
                        }),
                        None,
                    ))
                }
                // Bidirectional server→client requests need an answer
                // channel the typed client does not have. Skip rather than
                // fail: an activation may interleave them with real data.
                Ok(StreamItem::Request { .. }) => continue,
            }
        }
    }))
}

/// [`decode`] for a typed client's items (PERF.115-36): the same mapping,
/// each data item's content decoded straight from the wire's JSON into `E`,
/// never into a `Value` first. What a generated client calls, on
/// [`Transport::call_raw`](crate::Transport::call_raw)'s items. A decode error
/// says what [`decode`] says of the same content.
pub fn decode_raw<'a, E>(items: RawItemStream<'a>) -> EventStream<'a, E>
where
    E: DeserializeOwned + 'a,
{
    Box::pin(stream::unfold(Some(items), |state| async move {
        let mut items = state?;
        loop {
            return match items.next().await? {
                Err(e) => Some((Err(ClientError::Transport(e)), None)),
                Ok(RawItem::Done) => None,
                Ok(RawItem::Data { content, content_type }) => Some(match content.decode::<E>() {
                    Ok(event) => (Ok(ClientEvent::Data(event)), Some(items)),
                    Err(e) => (
                        Err(ClientError::Decode {
                            content_type: content_type.map(|c| c.to_string()),
                            message: message_of(&e),
                        }),
                        None,
                    ),
                }),
                Ok(RawItem::Progress { message, percentage }) => {
                    Some((Ok(ClientEvent::Progress { message, percentage }), Some(items)))
                }
                Ok(RawItem::Error { message, code, recoverable }) => {
                    Some((Err(ClientError::Remote { message, code, recoverable }), None))
                }
                Ok(RawItem::Request) => continue,
            };
        }
    }))
}

/// A decode error as [`decode`] words it: serde_json's message, without the
/// position a decode from text adds to it.
fn message_of(e: &serde_json::Error) -> String {
    let message = e.to_string();
    if e.line() == 0 {
        return message;
    }
    let at = format!(" at line {} column {}", e.line(), e.column());
    match message.strip_suffix(&at) {
        Some(bare) => bare.to_owned(),
        None => message,
    }
}

/// A typed stream that yields exactly one error and then ends.
///
/// What a generated client method returns when it cannot even build the
/// request — a parameter that would not serialize. Keeping the signature a
/// stream (rather than `Result<EventStream, _>`) means the call site handles
/// one error channel, not two.
pub fn error_stream<'a, E>(error: ClientError) -> EventStream<'a, E>
where
    E: 'a,
{
    Box::pin(stream::once(async move { Err(error) }))
}

/// Drive a typed stream to completion, collecting every `data` event.
///
/// The shape a unary method wants: menger's finite calls (`list`, `get`,
/// `blockers`, …) emit N data items then `done`, and the caller wants the
/// `Vec`. Progress events are discarded; the first error wins.
pub async fn collect_data<E, S>(mut stream: S) -> Result<Vec<E>, ClientError>
where
    S: Stream<Item = Result<ClientEvent<E>, ClientError>> + Unpin,
{
    let mut out = Vec::new();
    while let Some(item) = stream.next().await {
        match item? {
            ClientEvent::Data(e) => out.push(e),
            ClientEvent::Progress { .. } => {}
        }
    }
    Ok(out)
}
