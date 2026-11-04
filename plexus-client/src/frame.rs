//! What a client stream yields once its caller opts into progress.

use crate::Progress;

/// One frame of a call whose caller asked to see progress: an item, or a
/// progress report.
///
/// `I` is the method's item (a plain `T`, or a `Result<T, E>`); `P` is its
/// progress type ([`Note`](crate::Note) unless the method declares its own).
/// A caller that does not opt in never sees a `Frame` — its stream yields
/// `I` directly and progress is dropped.
///
/// Client-side only; never serialized.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Frame<I, P> {
    /// One of the method's items (a data frame).
    Item(I),
    /// A progress report (a progress frame).
    Progress(P),
}

impl<I, P> Frame<I, P> {
    /// `true` for [`Frame::Item`].
    pub fn is_item(&self) -> bool {
        matches!(self, Frame::Item(_))
    }

    /// `true` for [`Frame::Progress`].
    pub fn is_progress(&self) -> bool {
        matches!(self, Frame::Progress(_))
    }

    /// Borrow the item, if this is one.
    pub fn item(&self) -> Option<&I> {
        match self {
            Frame::Item(i) => Some(i),
            Frame::Progress(_) => None,
        }
    }

    /// The item, if this is one; a progress report is dropped.
    pub fn into_item(self) -> Option<I> {
        match self {
            Frame::Item(i) => Some(i),
            Frame::Progress(_) => None,
        }
    }

    /// The progress report, if this is one; an item is dropped.
    pub fn into_progress(self) -> Option<P> {
        match self {
            Frame::Progress(p) => Some(p),
            Frame::Item(_) => None,
        }
    }
}

/// What the server sends as `Progress<T, P>` arrives as `Frame<T, P>`:
/// `Update` ⇄ `Progress`, `Item` ⇄ `Item`.
impl<T, P> From<Progress<T, P>> for Frame<T, P> {
    fn from(p: Progress<T, P>) -> Self {
        match p {
            Progress::Update(p) => Frame::Progress(p),
            Progress::Item(t) => Frame::Item(t),
        }
    }
}
