//! `watch((a, b, c))` — several method streams in, one stream out whose item
//! is exactly the union of theirs (RETURNS-38).
//!
//! ```
//! use futures::{executor::block_on, stream, StreamExt};
//! use plexus_client::{watch, Watched3};
//!
//! # struct TicketsEvent; struct ChatsEvent; struct DeployEvent;
//! let tickets = stream::iter(vec![TicketsEvent]);
//! let chats = stream::iter(vec![ChatsEvent]);
//! let deployments = stream::iter(vec![DeployEvent]);
//!
//! let mut s = watch((tickets, chats, deployments));
//! block_on(async {
//!     while let Some(it) = s.next().await {
//!         // No wildcard: the item is exactly one of the three.
//!         match it {
//!             Watched3::A(TicketsEvent) => {}
//!             Watched3::B(ChatsEvent) => {}
//!             Watched3::C(DeployEvent) => {}
//!         }
//!     }
//! });
//! ```
//!
//! Rust has no variadics, so the streams go in as a tuple: [`Watch`] is
//! implemented for tuples of 2 to 12 streams, each giving its own
//! `WatchingN` stream of `WatchedN` items — one variant per element, in the
//! tuple's order, holding that stream's own item unchanged. A one-tuple is
//! the stream itself.
//!
//! # The rules
//!
//! | when | then |
//! |---|---|
//! | items arrive | they come out as each stream yields them; a stream that is ready is never starved (the merge polls round-robin) |
//! | one stream ends | it is dropped at once, and the merge goes on; it ends when all have |
//! | one stream yields an error | it is an item like any other (`Watched3::B(Err(e))` — the variant names its source); the others go on |
//! | [`Errors::Stop`] ([`watch_with`], or [`WatchingN::stop_on_error`](Watching3::stop_on_error)) | the failing item is the last: every other stream is dropped with it and the merge ends |
//! | the merge is dropped | every stream it still holds is dropped, once each — over a `WsTransport` that is one `call_unsub` per live subscription |
//!
//! Nothing is new on the wire: each stream is the call it already was.
//!
//! # The item is exact
//!
//! Widening the union does not compile:
//!
//! ```compile_fail
//! use futures::{executor::block_on, stream, StreamExt};
//! use plexus_client::{watch, Watched3};
//! struct A; struct B; struct C; struct Wide;
//! let mut s = watch((stream::iter(vec![A]), stream::iter(vec![B]), stream::iter(vec![C])));
//! let _: Option<Watched3<Wide, Wide, Wide>> = block_on(s.next());
//! ```
//!
//! Nor does watching nothing:
//!
//! ```compile_fail
//! let _ = plexus_client::watch(());
//! ```
//!
//! A stream that is not `Unpin` (an `async_stream` block, say) goes in
//! boxed: `Box::pin(s)`. Every generated client stream already is.

use futures_core::stream::{FusedStream, Stream};
use std::fmt;
use std::pin::Pin;
use std::task::{Context, Poll};

/// What the merge does when one of its streams yields an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Errors {
    /// The error is an item, and the other streams go on (the default).
    #[default]
    Continue,
    /// The error is the last item: the merge drops every stream it holds
    /// and ends.
    Stop,
}

/// An item that may be a failure — what [`Errors::Stop`] looks at.
/// Implemented for `Result`, which every client stream yields.
pub trait Fallible {
    fn is_failure(&self) -> bool;
}

impl<T, E> Fallible for Result<T, E> {
    fn is_failure(&self) -> bool {
        self.is_err()
    }
}

/// Several streams, given as a tuple, merged into one. See the
/// [module docs](self).
pub trait Watch {
    /// The merged stream.
    type Watching: Stream;
    fn watch(self) -> Self::Watching;
}

/// A merged stream that can be told to end at the first failure.
pub trait StopOnError {
    fn stop_on_error(self) -> Self;
}

/// Merge the streams of a tuple into one: `watch((a, b, c))`.
///
/// Its item is `Watched3<A::Item, B::Item, C::Item>` — exactly the three,
/// tagged by position; `watch((a,))` is `a` itself.
pub fn watch<W: Watch>(streams: W) -> W::Watching {
    streams.watch()
}

/// [`watch`], with the policy for an error said up front.
pub fn watch_with<W>(streams: W, errors: Errors) -> W::Watching
where
    W: Watch,
    W::Watching: StopOnError,
{
    let w = streams.watch();
    match errors {
        Errors::Continue => w,
        Errors::Stop => w.stop_on_error(),
    }
}

/// One stream is its own merge: its type unchanged.
impl<A: Stream> Watch for (A,) {
    type Watching = A;
    fn watch(self) -> A {
        self.0
    }
}

macro_rules! watch_arity {
    ($Watched:ident, $Watching:ident, $n:expr; $( ($V:ident, $f:ident, $i:expr) ),+ ) => {
        /// One item of a merged stream: which stream it came from (the
        /// variant, in the tuple's order) and that stream's own item.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $Watched<$($V),+> {
            $( $V($V), )+
        }

        impl<$($V),+> $Watched<$($V),+> {
            /// The source's position in the tuple, from 0.
            pub fn index(&self) -> usize {
                match self {
                    $( $Watched::$V(_) => $i, )+
                }
            }
        }

        impl<$($V: Fallible),+> Fallible for $Watched<$($V),+> {
            fn is_failure(&self) -> bool {
                match self {
                    $( $Watched::$V(x) => x.is_failure(), )+
                }
            }
        }

        /// The merge of a tuple of streams; see [`watch`].
        #[must_use = "streams do nothing unless polled"]
        pub struct $Watching<$($V: Stream),+> {
            $( $f: Option<$V>, )+
            /// The stream polled first next time — round-robin, so a busy
            /// stream cannot starve the others.
            next: usize,
            stop_if: Option<fn(&$Watched<$($V::Item),+>) -> bool>,
            ended: bool,
        }

        impl<$($V: Stream),+> $Watching<$($V),+> {
            /// End at the first failure: it is delivered as the last item,
            /// and every other stream is dropped with it.
            pub fn stop_on_error(mut self) -> Self
            where
                $( $V::Item: Fallible, )+
            {
                self.stop_if = Some(|w| w.is_failure());
                self
            }

            /// How many streams the merge still holds: each is let go when
            /// it is seen to end (polled to its end), or with the merge.
            pub fn live(&self) -> usize {
                0 $( + usize::from(self.$f.is_some()) )+
            }

            fn end(&mut self) {
                self.ended = true;
                $( self.$f = None; )+
            }
        }

        impl<$($V: Stream),+> StopOnError for $Watching<$($V),+>
        where
            $( $V::Item: Fallible, )+
        {
            fn stop_on_error(self) -> Self {
                $Watching::stop_on_error(self)
            }
        }

        impl<$($V: Stream),+> fmt::Debug for $Watching<$($V),+> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.debug_struct(stringify!($Watching))
                    .field("live", &self.live())
                    .field("stop_on_error", &self.stop_if.is_some())
                    .field("ended", &self.ended)
                    .finish()
            }
        }

        impl<$($V: Stream + Unpin),+> Watch for ($($V,)+) {
            type Watching = $Watching<$($V),+>;
            #[allow(non_snake_case)]
            fn watch(self) -> Self::Watching {
                let ($($V,)+) = self;
                $Watching {
                    $( $f: Some($V), )+
                    next: 0,
                    stop_if: None,
                    ended: false,
                }
            }
        }

        impl<$($V: Stream + Unpin),+> Stream for $Watching<$($V),+> {
            type Item = $Watched<$($V::Item),+>;

            fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
                let this = self.get_mut();
                if this.ended {
                    return Poll::Ready(None);
                }
                let start = this.next;
                this.next = (start + 1) % $n;
                for k in 0..$n {
                    let i = (start + k) % $n;
                    $(
                        if i == $i {
                            if let Some(s) = this.$f.as_mut() {
                                match Pin::new(s).poll_next(cx) {
                                    Poll::Ready(Some(x)) => {
                                        let item = $Watched::$V(x);
                                        if this.stop_if.is_some_and(|stop| stop(&item)) {
                                            this.end();
                                        }
                                        return Poll::Ready(Some(item));
                                    }
                                    // Ended: let it go now, not when the merge goes.
                                    Poll::Ready(None) => this.$f = None,
                                    Poll::Pending => {}
                                }
                            }
                        }
                    )+
                }
                if this.live() == 0 {
                    this.ended = true;
                    Poll::Ready(None)
                } else {
                    Poll::Pending
                }
            }

            fn size_hint(&self) -> (usize, Option<usize>) {
                let mut lo = 0usize;
                let mut hi = Some(0usize);
                $(
                    if let Some(s) = &self.$f {
                        let (l, h) = s.size_hint();
                        lo = lo.saturating_add(l);
                        hi = match (hi, h) {
                            (Some(a), Some(b)) => a.checked_add(b),
                            _ => None,
                        };
                    }
                )+
                if self.stop_if.is_some() {
                    lo = lo.min(1);
                }
                (lo, hi)
            }
        }

        impl<$($V: Stream + Unpin),+> FusedStream for $Watching<$($V),+> {
            fn is_terminated(&self) -> bool {
                self.ended
            }
        }
    };
}

watch_arity!(Watched2, Watching2, 2; (A, a, 0), (B, b, 1));
watch_arity!(Watched3, Watching3, 3; (A, a, 0), (B, b, 1), (C, c, 2));
watch_arity!(Watched4, Watching4, 4; (A, a, 0), (B, b, 1), (C, c, 2), (D, d, 3));
watch_arity!(Watched5, Watching5, 5; (A, a, 0), (B, b, 1), (C, c, 2), (D, d, 3), (E, e, 4));
watch_arity!(Watched6, Watching6, 6; (A, a, 0), (B, b, 1), (C, c, 2), (D, d, 3), (E, e, 4), (F, f, 5));
watch_arity!(Watched7, Watching7, 7; (A, a, 0), (B, b, 1), (C, c, 2), (D, d, 3), (E, e, 4), (F, f, 5), (G, g, 6));
watch_arity!(Watched8, Watching8, 8;
    (A, a, 0), (B, b, 1), (C, c, 2), (D, d, 3), (E, e, 4), (F, f, 5), (G, g, 6), (H, h, 7));
watch_arity!(Watched9, Watching9, 9;
    (A, a, 0), (B, b, 1), (C, c, 2), (D, d, 3), (E, e, 4), (F, f, 5), (G, g, 6), (H, h, 7), (I, i_, 8));
watch_arity!(Watched10, Watching10, 10;
    (A, a, 0), (B, b, 1), (C, c, 2), (D, d, 3), (E, e, 4), (F, f, 5), (G, g, 6), (H, h, 7), (I, i_, 8),
    (J, j, 9));
watch_arity!(Watched11, Watching11, 11;
    (A, a, 0), (B, b, 1), (C, c, 2), (D, d, 3), (E, e, 4), (F, f, 5), (G, g, 6), (H, h, 7), (I, i_, 8),
    (J, j, 9), (K, k, 10));
watch_arity!(Watched12, Watching12, 12;
    (A, a, 0), (B, b, 1), (C, c, 2), (D, d, 3), (E, e, 4), (F, f, 5), (G, g, 6), (H, h, 7), (I, i_, 8),
    (J, j, 9), (K, k, 10), (L, l, 11));
