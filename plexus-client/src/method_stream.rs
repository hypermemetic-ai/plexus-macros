//! What a many-item method's client call returns: [`MethodStream`].
//!
//! A method that yields many items (`-> impl Stream<Item = …>`) is called as
//! a stream of its items — a plain
//! `TryStream<Ok = I, Error = ClientError>` — and two per-call opt-ins turn
//! that same call into something else (RETURNS-1 decisions 6, 11):
//!
//! | Call | Yields | Progress frames |
//! |---|---|---|
//! | `client.m(..)` | `Result<I, ClientError>` | dropped, undecoded |
//! | `client.m(..).with_progress()` | `Result<Frame<I, P>, ClientError>`, in arrival order | decoded as `P` |
//! | `client.m(..).events()` | `Result<{Ns}Event, ClientError>`, `{Ns}Event::M(item)` | dropped |
//!
//! `I` is the method's item — a plain `T`, or a `Result<T, E>` decoded from
//! its `{"ok"}` / `{"err"}` wire form — `P` its progress type ([`NoProgress`]
//! when it declares none) and `A` the activation's `{Ns}Event` aggregate.
//!
//! # Decoding, exhaustively
//!
//! | Wire item | Yielded | Stream continues? |
//! |---|---|---|
//! | `data` (decodes as `I`) | the item | yes |
//! | `data` (does not) | `Err(ClientError::Decode)` | **no** |
//! | `progress` | nothing — unless opted in: `Frame::Progress(p)` | yes |
//! | `progress` (opted in, does not decode as `P`) | `Err(ClientError::Decode)` | **no** |
//! | `error` | `Err(ClientError::Remote(RemoteError))` | **no** |
//! | `request` | *skipped* (the typed client has no answer channel) | yes |
//! | `done` | nothing | **no** |
//! | transport failure | `Err(ClientError::Transport)` | **no** |
//!
//! A failed call stays an *item* of the stream, so a cut-off stream cannot
//! pass for a finished one. An `err` item (`Err(e)` of a Result method) is
//! data, the activation saying no: it is `Ok(Err(e))` here and does not end
//! the stream — the method decides whether more follows.
//!
//! [`NoProgress`] is uninhabited, so `.with_progress()` exists on every
//! many-item method and still never yields a progress frame where the schema
//! declares none: such a frame cannot decode, and is a contract break like
//! any other.

use std::fmt;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures_core::{FusedStream, Stream};
use serde::de::{self, DeserializeOwned, Deserializer};
use serde::{Deserialize, Serialize, Serializer};
use serde_json::Value;

use crate::envelope::StreamItem;
use crate::error::ClientError;
use crate::frame::Frame;
use crate::transport::{ItemStream, MethodRef, Transport};

/// Decodes one data frame's `content` into a method's item `I`.
///
/// A plain-`T` method's is `serde_json::from_value::<T>`; a Result method's
/// is [`wire_result::from_value::<T, E>`](crate::wire_result::from_value),
/// which reads `{"ok"}` / `{"err"}`. A function pointer, so it is `Send`,
/// `Sync` and `Copy` whatever `I` is.
pub type Decoder<I> = fn(Value) -> Result<I, serde_json::Error>;

/// The progress type of a method that reports none. Uninhabited: no value of
/// it exists, so a stream typed `Frame<I, NoProgress>` can only ever yield
/// items.
///
/// It deserializes from nothing — a progress frame for a method whose schema
/// declares no progress is a contract break, reported as
/// [`ClientError::Decode`] under `.with_progress()` (and dropped undecoded
/// without it, as every progress frame is).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NoProgress {}

impl Serialize for NoProgress {
    fn serialize<S: Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
        match *self {}
    }
}

impl<'de> Deserialize<'de> for NoProgress {
    fn deserialize<D: Deserializer<'de>>(_: D) -> Result<Self, D::Error> {
        Err(de::Error::custom(
            "a progress frame for a method that declares no progress",
        ))
    }
}

/// The wire items behind a typed stream, and where they stand.
enum State<'a> {
    /// Items still arriving.
    Live(ItemStream<'a>),
    /// The call failed before anything was sent (a parameter that would not
    /// encode); yielded once, then finished.
    Failed(ClientError),
    /// A terminal item was seen (or yielded). The wire stream is dropped the
    /// moment this is reached, so a transport that cancels on drop (the
    /// WebSocket one) cancels exactly the calls the client gave up on.
    Finished,
}

/// The decoding shared by [`MethodStream`] and [`WithProgress`].
struct Core<'a, I> {
    state: State<'a>,
    item: Decoder<I>,
}

impl<'a, I> Core<'a, I> {
    fn new(state: State<'a>, item: Decoder<I>) -> Self {
        Self { state, item }
    }

    /// The next frame. `progress` is `None` to drop progress frames
    /// undecoded, `Some(decoder)` to yield them.
    fn poll_frame<P>(
        &mut self,
        cx: &mut Context<'_>,
        progress: Option<Decoder<P>>,
    ) -> Poll<Option<Result<Frame<I, P>, ClientError>>> {
        loop {
            let next = match &mut self.state {
                State::Live(items) => match items.as_mut().poll_next(cx) {
                    Poll::Pending => return Poll::Pending,
                    Poll::Ready(next) => next,
                },
                State::Failed(_) => {
                    let State::Failed(e) = std::mem::replace(&mut self.state, State::Finished)
                    else {
                        unreachable!("matched Failed above")
                    };
                    return Poll::Ready(Some(Err(e)));
                }
                State::Finished => return Poll::Ready(None),
            };

            let (out, terminal) = match next {
                // Normal completion is the stream ending; a `done` is not an
                // item. A transport that ends without one has said all it
                // will say.
                None | Some(Ok(StreamItem::Done { .. })) => {
                    self.state = State::Finished;
                    return Poll::Ready(None);
                }
                Some(Err(e)) => (Err(ClientError::Transport(e)), true),
                Some(Ok(StreamItem::Error { error, .. })) => (Err(ClientError::Remote(error)), true),
                // Bidirectional server→client requests need an answer channel
                // the typed client does not have. Skip rather than fail: an
                // activation may interleave them with real data.
                Some(Ok(StreamItem::Request { .. })) => continue,
                Some(Ok(StreamItem::Data {
                    content,
                    content_type,
                    ..
                })) => match (self.item)(content) {
                    Ok(item) => (Ok(Frame::Item(item)), false),
                    Err(e) => (
                        Err(ClientError::Decode {
                            content_type,
                            message: e.to_string(),
                        }),
                        true,
                    ),
                },
                Some(Ok(StreamItem::Progress {
                    content,
                    content_type,
                    ..
                })) => match progress {
                    None => continue,
                    Some(decode) => match decode(content) {
                        Ok(p) => (Ok(Frame::Progress(p)), false),
                        Err(e) => (
                            Err(ClientError::Decode {
                                content_type,
                                message: format!("progress: {e}"),
                            }),
                            true,
                        ),
                    },
                },
            };
            if terminal {
                self.state = State::Finished;
            }
            return Poll::Ready(Some(out));
        }
    }

    fn is_terminated(&self) -> bool {
        matches!(self.state, State::Finished)
    }

    fn state_name(&self) -> &'static str {
        match self.state {
            State::Live(_) => "live",
            State::Failed(_) => "failed",
            State::Finished => "finished",
        }
    }
}

fn identity<T>(t: T) -> T {
    t
}

/// One call of a many-item method: a stream of its items.
///
/// `I` is the item, `P` the progress type ([`NoProgress`] when the method
/// declares none), `A` the activation's `{Ns}Event` aggregate. A generated
/// client spells all four — `MethodStream<'_, Result<Row, ScanError>,
/// NoProgress, TicketsEvent>`; the defaults are for the macro-free
/// [`decode`](crate::decode) path.
///
/// It is a plain `Stream<Item = Result<I, ClientError>>` — a `TryStream` —
/// so `futures`' combinators apply:
///
/// ```ignore
/// use futures::TryStreamExt;
/// let rows: Vec<Result<Row, ScanError>> = client.scan("a".into()).try_collect().await?;
/// ```
///
/// Progress frames are dropped undecoded unless the caller opts in with
/// [`with_progress`](Self::with_progress); [`events`](Self::events) turns
/// the call into the activation's aggregate. See the [module docs](self)
/// for the decoding table.
///
/// `Unpin`, and `Send` wherever the transport's item stream is (everywhere
/// but wasm32); dropping it mid-stream drops the wire stream, which cancels
/// the call on a transport that can (the WebSocket one unsubscribes).
pub struct MethodStream<'a, I, P = NoProgress, A = I> {
    core: Core<'a, I>,
    event: fn(I) -> A,
    _progress: PhantomData<fn() -> P>,
}

impl<'a, I, P, A> MethodStream<'a, I, P, A> {
    /// Type a transport's wire items as one method's stream: `item` decodes
    /// each data frame's `content`, `event` wraps an item in the
    /// aggregate (`TicketsEvent::List`) for [`events`](Self::events).
    pub fn new(items: ItemStream<'a>, item: Decoder<I>, event: fn(I) -> A) -> Self {
        Self::from_core(Core::new(State::Live(items), item), event)
    }

    /// Call `method` on `transport` with `params` and type the answer — what
    /// a generated client's many-item method is. When `params` is an `Err`
    /// (an argument that would not encode) nothing is sent and the stream
    /// yields that error once.
    pub fn call<T>(
        transport: &'a T,
        method: MethodRef,
        params: Result<Value, ClientError>,
        item: Decoder<I>,
        event: fn(I) -> A,
    ) -> Self
    where
        T: Transport + ?Sized,
    {
        let state = match params {
            Ok(params) => State::Live(transport.call(method, params)),
            Err(e) => State::Failed(e),
        };
        Self::from_core(Core::new(state, item), event)
    }

    fn from_core(core: Core<'a, I>, event: fn(I) -> A) -> Self {
        Self {
            core,
            event,
            _progress: PhantomData,
        }
    }

    /// Opt in to progress: the same call, yielding each item as
    /// [`Frame::Item`] and each progress frame, decoded as `P`, as
    /// [`Frame::Progress`] — in arrival order.
    ///
    /// For a method that declares no progress `P` is [`NoProgress`], and the
    /// stream yields only items.
    pub fn with_progress(self) -> WithProgress<'a, I, P>
    where
        P: DeserializeOwned,
    {
        WithProgress {
            core: self.core,
            _progress: PhantomData,
        }
    }

    /// The same call as the activation's aggregate: each item `i` becomes
    /// `{Ns}Event::M(i)`; progress is dropped (the aggregate carries items
    /// only). Every method's `events()` has the same type, so several calls
    /// merge into one listener with ordinary stream plumbing:
    ///
    /// ```ignore
    /// let mut all = futures::stream::select_all([
    ///     client.watch(None).events(),
    ///     client.list(None, None).events(),
    /// ]);
    /// while let Some(ev) = all.next().await { state.reduce(ev?) }   // ev: TicketsEvent
    /// ```
    pub fn events(self) -> Events<'a, A>
    where
        I: 'a,
        P: 'a,
        A: 'a,
    {
        let event = self.event;
        Events {
            inner: Box::pin(futures_util::StreamExt::map(self, move |r| r.map(event))),
        }
    }
}

impl<I, P, A> Stream for MethodStream<'_, I, P, A> {
    type Item = Result<I, ClientError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.get_mut()
            .core
            .poll_frame::<NoProgress>(cx, None)
            .map(|next| {
                next.map(|frame| {
                    frame.map(|frame| match frame {
                        Frame::Item(item) => item,
                        Frame::Progress(never) => match never {},
                    })
                })
            })
    }
}

impl<I, P, A> FusedStream for MethodStream<'_, I, P, A> {
    fn is_terminated(&self) -> bool {
        self.core.is_terminated()
    }
}

impl<I, P, A> fmt::Debug for MethodStream<'_, I, P, A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MethodStream")
            .field("state", &self.core.state_name())
            .finish_non_exhaustive()
    }
}

/// A [`MethodStream`] whose caller opted in to progress
/// ([`MethodStream::with_progress`]): `Result<Frame<I, P>, ClientError>`,
/// in arrival order.
pub struct WithProgress<'a, I, P> {
    core: Core<'a, I>,
    _progress: PhantomData<fn() -> P>,
}

impl<I, P: DeserializeOwned> Stream for WithProgress<'_, I, P> {
    type Item = Result<Frame<I, P>, ClientError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.get_mut()
            .core
            .poll_frame(cx, Some(serde_json::from_value::<P> as Decoder<P>))
    }
}

impl<I, P: DeserializeOwned> FusedStream for WithProgress<'_, I, P> {
    fn is_terminated(&self) -> bool {
        self.core.is_terminated()
    }
}

impl<I, P> fmt::Debug for WithProgress<'_, I, P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WithProgress")
            .field("state", &self.core.state_name())
            .finish_non_exhaustive()
    }
}

#[cfg(not(target_arch = "wasm32"))]
type BoxedEvents<'a, A> = Pin<Box<dyn FusedStream<Item = Result<A, ClientError>> + Send + 'a>>;

#[cfg(target_arch = "wasm32")]
type BoxedEvents<'a, A> = Pin<Box<dyn FusedStream<Item = Result<A, ClientError>> + 'a>>;

/// A call as the activation's aggregate ([`MethodStream::events`]):
/// `Result<A, ClientError>`, where `A` is `{Ns}Event`.
///
/// Its type names only the aggregate, not the method, so every method's
/// `events()` is the same type and several calls merge with
/// `futures::stream::select_all` (or `select`). Mirrors the transport's
/// `Send` bound: `Send` off wasm32, not on it.
pub struct Events<'a, A> {
    inner: BoxedEvents<'a, A>,
}

impl<A> Stream for Events<'_, A> {
    type Item = Result<A, ClientError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.inner.as_mut().poll_next(cx)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<A> FusedStream for Events<'_, A> {
    fn is_terminated(&self) -> bool {
        self.inner.is_terminated()
    }
}

impl<A> fmt::Debug for Events<'_, A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Events")
            .field("terminated", &self.inner.is_terminated())
            .finish_non_exhaustive()
    }
}

/// Call a one-item method (`-> T`, `-> Result<T, E>`) and return its item —
/// what a generated client's one-item `async fn` is.
///
/// Reads the call to its terminal frame: one data item, then `done`. So a
/// transport that cancels a dropped call (the WebSocket one) never cancels a
/// call that finished, and a server that breaks the contract is caught:
///
/// | Wire | Result |
/// |---|---|
/// | one `data` (decodes), `done` | `Ok(item)` |
/// | `error` (before or after the item) | `Err(ClientError::Remote)` |
/// | `done` with no item, or a second item | `Err(ClientError::Decode)` naming the method |
/// | transport failure | `Err(ClientError::Transport)` |
///
/// Progress frames are dropped (a one-item method never reports progress:
/// RETURNS-1 decision 10); `request` frames are skipped. When `params` is an
/// `Err` nothing is sent and that is the result.
pub async fn call_one<T, I>(
    transport: &T,
    method: MethodRef,
    params: Result<Value, ClientError>,
    item: Decoder<I>,
) -> Result<I, ClientError>
where
    T: Transport + ?Sized,
{
    use futures_util::StreamExt;

    let mut items: MethodStream<'_, I> =
        MethodStream::call(transport, method, params, item, identity::<I>);
    let contract = |message: &str| ClientError::Decode {
        content_type: Some(method.qualified()),
        message: message.to_string(),
    };
    let first = match items.next().await {
        Some(first) => first?,
        None => return Err(contract("a one-item method ended without its item")),
    };
    match items.next().await {
        None => Ok(first),
        Some(Err(e)) => Err(e),
        Some(Ok(_)) => Err(contract("a one-item method sent more than one item")),
    }
}

/// The `Decoder` for a plain item, as a function pointer.
pub(crate) fn plain<I: DeserializeOwned>() -> Decoder<I> {
    serde_json::from_value::<I>
}

/// `MethodStream::new` with the identity aggregate, for the macro-free path.
pub(crate) fn untagged<'a, I>(items: ItemStream<'a>, item: Decoder<I>) -> MethodStream<'a, I> {
    MethodStream::new(items, item, identity::<I>)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_progress_never_deserializes() {
        let err = serde_json::from_value::<NoProgress>(serde_json::json!({"message": "x"}))
            .unwrap_err();
        assert!(err.to_string().contains("declares no progress"), "{err}");
    }

    #[test]
    fn method_streams_are_send_and_unpin() {
        fn send_unpin<T: Send + Unpin>() {}
        send_unpin::<MethodStream<'static, String, NoProgress, u8>>();
        send_unpin::<WithProgress<'static, String, crate::Note>>();
        send_unpin::<Events<'static, u8>>();
    }
}
