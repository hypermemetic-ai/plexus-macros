//! MACRO-13 · what a trait the activation macro emits leans on.
//!
//! `#[plexus_macros::activation(trait)]` on `impl FooActivation` emits
//! `trait Foo`: every wire method as `fn(&self, ..) -> BoxStream<'static, E>`,
//! implemented by the activation and by `FooClient<T>`. A client call is a
//! stream of `Result<ClientEvent<E>, ClientError>`; a trait method is a
//! stream of `E`. This module is the bridge:
//!
//! - [`FromClientError`] — how a client failure becomes the event type's own
//!   in-band error (one line, usually `Error { message }`), so a caller of
//!   `dyn Foo` sees one shape whether it is talking to a local
//!   implementation or a remote one;
//! - [`owned_call`] — a `'static` call from an owned transport, since a trait
//!   method's stream outlives the `&self` it was called on;
//! - [`into_events`] / [`error_events`] — the mapping itself: a data event is
//!   yielded, a progress report is dropped, an error is yielded as
//!   `E::from_client_error(err)` and ends the stream.

use crate::decode::EventStream;
use crate::envelope::{RawItem, StreamItem};
use crate::error::{ClientError, TransportError};
use crate::event::ClientEvent;
use crate::transport::{ItemStream, MethodRef, RawItemStream, Transport};
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
impl FromClientError for String {
    fn from_client_error(error: ClientError) -> Self {
        error.to_string()
    }
}

/// A typed client stream as a trait method's stream: data events yielded,
/// progress dropped, the first error yielded as `E::from_client_error(err)`
/// and the stream ended there.
pub fn into_events<'a, E>(events: EventStream<'a, E>) -> BoxStream<'a, E>
where
    E: FromClientError + 'a,
{
    Box::pin(stream::unfold(Some(events), |state| async move {
        let mut events = state?;
        loop {
            match events.next().await? {
                Ok(ClientEvent::Data(e)) => return Some((e, Some(events))),
                Ok(ClientEvent::Progress { .. }) => continue,
                Err(err) => return Some((E::from_client_error(err), None)),
            }
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

type Slot<I> = Arc<Mutex<Option<Result<I, TransportError>>>>;

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
    let slot: Slot<StreamItem> = Arc::new(Mutex::new(None));
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

/// [`owned_call`] for a typed client: `Transport::call_raw` on a transport
/// the stream owns (PERF.115-36).
#[cfg(not(target_arch = "wasm32"))]
pub fn owned_call_raw<T>(transport: T, method: MethodRef, params: Value) -> RawItemStream<'static>
where
    T: Transport + Send + Sync + 'static,
{
    let slot: Slot<RawItem> = Arc::new(Mutex::new(None));
    let tx = slot.clone();
    let driver: Driver = Box::pin(async move {
        let transport = transport;
        let mut items = transport.call_raw(method, params);
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
    let slot: Slot<StreamItem> = Arc::new(Mutex::new(None));
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

/// `owned_call_raw` on an owned transport. See the non-wasm32 variant.
#[cfg(target_arch = "wasm32")]
pub fn owned_call_raw<T>(transport: T, method: MethodRef, params: Value) -> RawItemStream<'static>
where
    T: Transport + 'static,
{
    let slot: Slot<RawItem> = Arc::new(Mutex::new(None));
    let tx = slot.clone();
    let driver: Driver = Box::pin(async move {
        let transport = transport;
        let mut items = transport.call_raw(method, params);
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

struct Owned<I> {
    driver: Option<Driver>,
    slot: Slot<I>,
}

impl<I> Owned<I> {
    fn take_item(&self) -> Option<Result<I, TransportError>> {
        self.slot.lock().unwrap_or_else(|p| p.into_inner()).take()
    }
}

impl<I> Stream for Owned<I> {
    type Item = Result<I, TransportError>;

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
                StreamItem::Progress { metadata: None, message: "half".into(), percentage: Some(50.0) },
                StreamItem::Data { metadata: None, content_type: None, content: serde_json::json!({"N": 2}) },
                StreamItem::Error { metadata: None, message: "no".into(), code: None, recoverable: false },
                StreamItem::Data { metadata: None, content_type: None, content: serde_json::json!({"N": 3}) },
            ],
        );
        let s = into_events::<Ev>(decode(owned_call(Arc::new(t), COUNT, serde_json::json!({}))));
        let got: Vec<Ev> = futures::executor::block_on(s.collect());
        assert_eq!(got, vec![Ev::N(1), Ev::N(2), Ev::Error("remote error: no".into())]);
    }
}
