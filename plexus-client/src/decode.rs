//! The macro-free typed path: a transport's wire items, typed by hand.
//!
//! A generated client builds its streams with [`MethodStream::call`] and
//! [`call_one`](crate::call_one). These are the same decoding for a caller
//! holding a raw [`ItemStream`] — a test, a tool that reads a stream the
//! macro did not generate a client for — with the item type spelled at the
//! call site:
//!
//! ```
//! use futures::TryStreamExt;
//! use plexus_client::{decode, MemoryTransport, MethodRef, Transport};
//!
//! #[derive(serde::Serialize, serde::Deserialize, Debug, PartialEq)]
//! struct Tick { n: u32 }
//!
//! const COUNT: MethodRef = MethodRef::new("demo", "count");
//! let t = MemoryTransport::new().script_events(COUNT, &[Tick { n: 1 }, Tick { n: 2 }]);
//!
//! let ticks: Vec<Tick> = futures::executor::block_on(
//!     decode::<Tick>(t.call(COUNT, serde_json::json!({}))).try_collect(),
//! )
//! .unwrap();
//! assert_eq!(ticks, vec![Tick { n: 1 }, Tick { n: 2 }]);
//! ```
//!
//! The decoding table is [`method_stream`](crate::method_stream)'s.

use serde::de::DeserializeOwned;

use crate::method_stream::{plain, untagged, MethodStream, WithProgress};
use crate::transport::ItemStream;

/// A plain-`T` method's items: each data frame's `content` decodes as `I`;
/// progress frames are dropped. The stream's aggregate is `I` itself
/// ([`MethodStream::events`] yields the items).
pub fn decode<'a, I>(items: ItemStream<'a>) -> MethodStream<'a, I>
where
    I: DeserializeOwned,
{
    untagged(items, plain::<I>())
}

/// [`decode`], opted in to progress: items as `Frame::Item(i)`, progress
/// frames decoded as `P` and yielded as `Frame::Progress(p)`, in arrival
/// order.
pub fn decode_with_progress<'a, I, P>(items: ItemStream<'a>) -> WithProgress<'a, I, P>
where
    I: DeserializeOwned,
    P: DeserializeOwned,
{
    MethodStream::<'a, I, P, I>::new(items, plain::<I>(), |i| i).with_progress()
}

/// A Result method's items: each data frame's `content` is `{"ok": t}` or
/// `{"err": e}` ([`wire_result`](crate::wire_result)) and decodes as `Ok(t)`
/// / `Err(e)`. An `Err` is data — the activation saying no — so it does not
/// end the stream.
pub fn decode_result<'a, T, E>(items: ItemStream<'a>) -> MethodStream<'a, Result<T, E>>
where
    T: DeserializeOwned,
    E: DeserializeOwned,
{
    untagged(items, crate::wire_result::from_value::<T, E>)
}
