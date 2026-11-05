//! What a trait the activation macro emits leans on (MACRO-13, RETURNS-36),
//! and what is left of the client-implements-the-trait rule (RETURNS-47).
//!
//! `#[plexus_macros::activation(trait = "Foo")]` (or a bodiless contract)
//! emits `trait Foo`, each method under its own return type, boxed: a
//! one-item method's a [`BoxFuture`] of its item, a stream form's a
//! [`BoxStream`] of its items. The activation implements it; the client does
//! not (RETURNS-47, superseding RETURNS-43): its methods answer
//! `Result<T, ClientError>` or a `MethodStream`, so a client failure is a
//! `Result` its caller handles, never a value folded into `T`, and every item
//! type works — `u64`, `bool`, `Vec<T>` — with nothing to implement.
//!
//! [`FromClientError`] is what that rule asked of every item type. Nothing
//! the macro emits asks for it any more; it stays for the code that
//! implements it.

use crate::error::ClientError;
use futures_core::Stream;
use std::future::Future;
use std::pin::Pin;

/// A boxed stream of `T` — the return type of every method of a trait the
/// activation macro emits. `Send` off wasm32, as [`crate::ItemStream`] is; on
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

/// `Send` off wasm32, nothing on it: the bound a client puts on what it
/// moves into a boxed future or stream (an `Input`'s items, BIDIR-6).
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
/// RETURNS-47: no longer required of anything — the generated client
/// implements no trait and answers `Result<T, ClientError>` — and kept for
/// the code that implements it. Before, the client's implementation of an
/// emitted trait yielded a client failure as `E::from_client_error(err)`.
/// An event enum carried it as its in-band error variant:
///
/// ```
/// # #[derive(Debug)] enum EchoEvent { Said { text: String }, Error { message: String } }
/// impl plexus_client::FromClientError for EchoEvent {
///     fn from_client_error(error: plexus_client::ClientError) -> Self {
///         EchoEvent::Error { message: error.to_string() }
///     }
/// }
/// ```
pub trait FromClientError {
    /// The event that stands for `error`. It is the last item of its stream.
    fn from_client_error(error: ClientError) -> Self;
}

/// A Result item carries a client failure as its error (RETURNS-36).
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

/// A string carries a client failure as its message.
impl FromClientError for String {
    fn from_client_error(error: ClientError) -> Self {
        error.to_string()
    }
}
