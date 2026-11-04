//! MACRO-13 · what a trait the activation macro emits leans on.
//!
//! `#[plexus_macros::activation(trait)]` on `impl FooActivation` emits
//! `trait Foo`: every wire method as `fn(&self, ..) -> BoxStream<'static, E>`,
//! implemented by the activation and by `FooClient<T>`. A client call is a
//! stream of `Result<E, ClientError>` (RETURNS-9's `MethodStream`, progress
//! already dropped); a trait method is a stream of `E`. This module is the
//! bridge:
//!
//! - [`FromClientError`] — how a client failure becomes the event type's own
//!   in-band error (one line, usually `Error { message }`), so a caller of
//!   `dyn Foo` sees one shape whether it is talking to a local
//!   implementation or a remote one;
//! - [`owned_call`] — a `'static` call from an owned transport, since a trait
//!   method's stream outlives the `&self` it was called on;
//! - [`into_events`] / [`error_events`] — the mapping itself: an item is
//!   yielded, an error is yielded as `E::from_client_error(err)` and ends the
//!   stream.
//!
//! RETURNS-36: the trait is always emitted, each method under its own return
//! type: a one-item method's is a [`BoxFuture`] of its item, a stream form's a
//! [`BoxStream`] of its items. [`trait_one`], [`trait_many`],
//! [`trait_progress`] and [`trait_result_progress`] are the client half's.

use crate::envelope::StreamItem;
use crate::error::{ClientError, TransportError};
use crate::transport::{ItemStream, MethodRef, Transport};
use futures_core::Stream;
use futures_util::{stream, StreamExt};
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

/// A boxed stream of `T` — the return type of every method of a trait the
/// activation macro emits. `Send` off wasm32, as [`ItemStream`] is; on
/// wasm32 the browser's futures are `!Send`, so the bound is dropped.
#[cfg(not(target_arch = "wasm32"))]
pub type BoxStream<'a, T> = Pin<Box<dyn Stream<Item = T> + Send + 'a>>;

/// A boxed stream of `T`. See the non-wasm32 variant.
#[cfg(target_arch = "wasm32")]
pub type BoxStream<'a, T> = Pin<Box<dyn Stream<Item = T> + 'a>>;

/// A boxed future of `T` — what a one-item method of an emitted trait
/// returns (RETURNS-36). `Send` off wasm32, as [`BoxStream`] is.
#[cfg(not(target_arch = "wasm32"))]
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// A boxed future of `T`. See the non-wasm32 variant.
#[cfg(target_arch = "wasm32")]
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

/// `Send` off wasm32, nothing on it: the bound a trait's client half puts
/// on what it moves into a boxed future or stream.
#[cfg(not(target_arch = "wasm32"))]
pub trait MaybeSend: Send {}
#[cfg(not(target_arch = "wasm32"))]
impl<T: Send> MaybeSend for T {}
/// See the non-wasm32 variant.
#[cfg(target_arch = "wasm32")]
pub trait MaybeSend {}
#[cfg(target_arch = "wasm32")]
impl<T> MaybeSend for T {}

/// An event type that can carry a client failure in band.
///
/// The client's implementation of an emitted trait yields each data event
/// as is and, on an `Err(ClientError)`, yields
/// `E::from_client_error(err)` and ends. Implement it on the activation's
/// event enum, usually as its in-band error variant:
///
/// ```
/// # #[derive(Debug)] enum EchoEvent { Said { text: String }, Error { message: String } }
/// impl plexus_client::FromClientError for EchoEvent {
///     fn from_client_error(error: plexus_client::ClientError) -> Self {
///         EchoEvent::Error { message: error.to_string() }
///     }
/// }
/// ```
#[diagnostic::on_unimplemented(
    message = "`{Self}` does not implement `plexus_client::FromClientError`",
    label = "the event type of a trait emitted by `#[plexus_macros::activation]` must implement `plexus_client::FromClientError`",
    note = "the client's implementation of the trait yields a client failure as `{Self}::from_client_error(err)`; implement `plexus_client::FromClientError` for `{Self}` (usually its in-band `Error {{ message }}` variant)"
)]
pub trait FromClientError {
    /// The event that stands for `error`. It is the last item of its stream.
    fn from_client_error(error: ClientError) -> Self;
}

/// A stream of strings carries a client failure as its message — so an
/// activation whose events are plain `String`s (a type no crate but this one
/// may implement the trait for) still has its trait implemented by its client.
/// A Result item carries a client failure as its error (RETURNS-36): what
/// the trait's client half yields for a fallible method.
impl<T, E: FromClientError> FromClientError for Result<T, E> {
    fn from_client_error(error: ClientError) -> Self {
        Err(E::from_client_error(error))
    }
}

/// A progress item carries a client failure as its item (RETURNS-36).
impl<T: FromClientError, P> FromClientError for crate::progress::Progress<T, P> {
    fn from_client_error(error: ClientError) -> Self {
        crate::progress::Progress::Item(T::from_client_error(error))
    }
}

impl FromClientError for String {
    fn from_client_error(error: ClientError) -> Self {
        error.to_string()
    }
}

/// A typed client stream as a trait method's stream: data events yielded,
/// progress dropped, the first error yielded as `E::from_client_error(err)`
/// and the stream ended there.
#[cfg(not(target_arch = "wasm32"))]
pub fn into_events<'a, E, S>(events: S) -> BoxStream<'a, E>
where
    E: FromClientError + Send + 'a,
    S: Stream<Item = Result<E, ClientError>> + Unpin + Send + 'a,
{
    Box::pin(stream::unfold(Some(events), |state| async move {
        let mut events = state?;
        match events.next().await? {
            Ok(e) => Some((e, Some(events))),
            Err(err) => Some((E::from_client_error(err), None)),
        }
    }))
}

/// [`into_events`] on wasm32, where a stream need not be `Send`.
#[cfg(target_arch = "wasm32")]
pub fn into_events<'a, E, S>(events: S) -> BoxStream<'a, E>
where
    E: FromClientError + 'a,
    S: Stream<Item = Result<E, ClientError>> + Unpin + 'a,
{
    Box::pin(stream::unfold(Some(events), |state| async move {
        let mut events = state?;
        match events.next().await? {
            Ok(e) => Some((e, Some(events))),
            Err(err) => Some((E::from_client_error(err), None)),
        }
    }))
}

/// A trait method's stream that yields `E::from_client_error(error)` and
/// ends — what an emitted client method returns when a parameter would not
/// serialize.
pub fn error_events<'a, E>(error: ClientError) -> BoxStream<'a, E>
where
    E: FromClientError + 'a,
{
    Box::pin(stream::once(async move { E::from_client_error(error) }))
}

// ---------------------------------------------------------------------------
// RETURNS-36: the trait's client half, one helper per signature form. Each
// takes its call already made (`owned_call`, or a params error) and yields
// the method's own return type, a client failure folded in by
// `FromClientError`.
// ---------------------------------------------------------------------------

/// `-> T` / `-> Result<T, E>`: one item.
pub fn trait_one<T, I>(
    transport: T,
    method: MethodRef,
    params: Result<Value, ClientError>,
    item: crate::method_stream::Decoder<I>,
) -> BoxFuture<'static, I>
where
    T: Transport + MaybeSend + Sync + 'static,
    I: FromClientError + MaybeSend + 'static,
{
    Box::pin(async move {
        match crate::method_stream::call_one(&transport, method, params, item).await {
            Ok(i) => i,
            Err(e) => I::from_client_error(e),
        }
    })
}

/// `-> impl Stream<Item = T>` / `Item = Result<T, E>`: many items.
pub fn trait_many<I>(
    items: Result<ItemStream<'static>, ClientError>,
    item: crate::method_stream::Decoder<I>,
) -> BoxStream<'static, I>
where
    I: FromClientError + MaybeSend + 'static,
{
    match items {
        Ok(items) => fold(crate::method_stream::MethodStream::<'static, I>::new(items, item, std::convert::identity)),
        Err(e) => Box::pin(stream::once(async move { I::from_client_error(e) })),
    }
}

/// `-> impl Stream<Item = Progress<T, P>>`: items and progress updates.
pub fn trait_progress<T, P>(
    items: Result<ItemStream<'static>, ClientError>,
    item: crate::method_stream::Decoder<T>,
) -> BoxStream<'static, crate::progress::Progress<T, P>>
where
    T: FromClientError + MaybeSend + 'static,
    P: serde::de::DeserializeOwned + MaybeSend + 'static,
{
    use crate::frame::Frame;
    use crate::progress::Progress;
    match items {
        Ok(items) => fold(
            crate::method_stream::MethodStream::<'static, T, P, T>::new(items, item, std::convert::identity)
                .with_progress()
                .map(|r| {
                    r.map(|f| match f {
                        Frame::Item(t) => Progress::Item(t),
                        Frame::Progress(p) => Progress::Update(p),
                    })
                }),
        ),
        Err(e) => Box::pin(stream::once(async move { Progress::Item(T::from_client_error(e)) })),
    }
}

/// `-> impl Stream<Item = Result<Progress<T, P>, E>>`.
pub fn trait_result_progress<T, P, E>(
    items: Result<ItemStream<'static>, ClientError>,
    item: crate::method_stream::Decoder<Result<T, E>>,
) -> BoxStream<'static, Result<crate::progress::Progress<T, P>, E>>
where
    T: MaybeSend + 'static,
    E: FromClientError + MaybeSend + 'static,
    P: serde::de::DeserializeOwned + MaybeSend + 'static,
{
    use crate::frame::Frame;
    use crate::progress::Progress;
    match items {
        Ok(items) => fold(
            crate::method_stream::MethodStream::<'static, Result<T, E>, P, Result<T, E>>::new(
                items,
                item,
                std::convert::identity,
            )
            .with_progress()
            .map(|r| {
                r.map(|f| match f {
                    Frame::Item(Ok(t)) => Ok(Progress::Item(t)),
                    Frame::Item(Err(e)) => Err(e),
                    Frame::Progress(p) => Ok(Progress::Update(p)),
                })
            }),
        ),
        Err(e) => Box::pin(stream::once(async move { Err(E::from_client_error(e)) })),
    }
}

/// A client stream as a trait method's: each item as is, the first failure
/// folded into the item type and the end.
fn fold<I, S>(events: S) -> BoxStream<'static, I>
where
    I: FromClientError + MaybeSend + 'static,
    S: Stream<Item = Result<I, ClientError>> + Unpin + MaybeSend + 'static,
{
    Box::pin(stream::unfold(Some(events), |state| async move {
        let mut events = state?;
        match events.next().await? {
            Ok(e) => Some((e, Some(events))),
            Err(err) => Some((I::from_client_error(err), None)),
        }
    }))
}

type Slot = Arc<Mutex<Option<Result<StreamItem, TransportError>>>>;

#[cfg(not(target_arch = "wasm32"))]
type Driver = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;
#[cfg(target_arch = "wasm32")]
type Driver = Pin<Box<dyn Future<Output = ()> + 'static>>;

/// `Transport::call` on a transport the stream owns, so the stream is
/// `'static`: a trait method returns a stream that outlives the `&self` it
/// was called on, and `Transport::call` borrows its transport.
///
/// The transport and the call's stream (which borrows it) live together in
/// one future, which hands each item out through a slot and pauses until it
/// is taken — a generator without `unsafe`.
#[cfg(not(target_arch = "wasm32"))]
pub fn owned_call<T>(transport: T, method: MethodRef, params: Value) -> ItemStream<'static>
where
    T: Transport + Send + Sync + 'static,
{
    let slot: Slot = Arc::new(Mutex::new(None));
    let tx = slot.clone();
    let driver: Driver = Box::pin(async move {
        let transport = transport;
        let mut items = transport.call(method, params);
        while let Some(item) = items.next().await {
            *tx.lock().unwrap_or_else(|p| p.into_inner()) = Some(item);
            HandOff(false).await;
        }
    });
    Box::pin(Owned { driver: Some(driver), slot })
}

/// `Transport::call` on an owned transport. See the non-wasm32 variant.
#[cfg(target_arch = "wasm32")]
pub fn owned_call<T>(transport: T, method: MethodRef, params: Value) -> ItemStream<'static>
where
    T: Transport + 'static,
{
    let slot: Slot = Arc::new(Mutex::new(None));
    let tx = slot.clone();
    let driver: Driver = Box::pin(async move {
        let transport = transport;
        let mut items = transport.call(method, params);
        while let Some(item) = items.next().await {
            *tx.lock().unwrap_or_else(|p| p.into_inner()) = Some(item);
            HandOff(false).await;
        }
    });
    Box::pin(Owned { driver: Some(driver), slot })
}

/// Pending once, so the stream can hand out the item just put in the slot;
/// ready the next time it is polled. It does not wake: the stream returns
/// the item, and the caller polls again when it wants the next one.
struct HandOff(bool);

impl Future for HandOff {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
        if self.0 {
            Poll::Ready(())
        } else {
            self.0 = true;
            Poll::Pending
        }
    }
}

struct Owned {
    driver: Option<Driver>,
    slot: Slot,
}

impl Owned {
    fn take_item(&self) -> Option<Result<StreamItem, TransportError>> {
        self.slot.lock().unwrap_or_else(|p| p.into_inner()).take()
    }
}

impl Stream for Owned {
    type Item = Result<StreamItem, TransportError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let Some(driver) = self.driver.as_mut() else {
            return Poll::Ready(None);
        };
        match driver.as_mut().poll(cx) {
            Poll::Ready(()) => {
                self.driver = None;
                Poll::Ready(self.take_item())
            }
            Poll::Pending => match self.take_item() {
                Some(item) => Poll::Ready(Some(item)),
                None => Poll::Pending,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode::decode;
    use crate::memory::MemoryTransport;

    #[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
    enum Ev {
        N(u8),
        Error(String),
    }

    impl FromClientError for Ev {
        fn from_client_error(error: ClientError) -> Self {
            Ev::Error(error.to_string())
        }
    }

    const COUNT: MethodRef = MethodRef::new("demo", "count");

    #[test]
    fn an_owned_call_outlives_its_caller_and_ends_on_an_error() {
        let t = MemoryTransport::new().script(
            COUNT,
            vec![
                StreamItem::Data { metadata: None, content_type: None, content: serde_json::json!({"N": 1}) },
                StreamItem::Progress { metadata: None, content_type: None, content: serde_json::json!({"message": "half", "percentage": 50.0}) },
                StreamItem::Data { metadata: None, content_type: None, content: serde_json::json!({"N": 2}) },
                StreamItem::Error {
                    metadata: None,
                    recoverable: false,
                    message: Some("no".into()),
                    code: None,
                    error: crate::RemoteError::Execution { message: "no".into() },
                    fields: Default::default(),
                },
                StreamItem::Data { metadata: None, content_type: None, content: serde_json::json!({"N": 3}) },
            ],
        );
        let s = into_events::<Ev, _>(decode::<Ev>(owned_call(Arc::new(t), COUNT, serde_json::json!({}))));
        let got: Vec<Ev> = futures::executor::block_on(s.collect());
        assert_eq!(got, vec![Ev::N(1), Ev::N(2), Ev::Error("remote error: no".into())]);
    }
}
