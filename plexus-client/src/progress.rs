//! Progress as part of a method's return type.
//!
//! A method that reports progress says so in its signature: its item is
//! `Progress<T, P>` — `impl Stream<Item = Result<Progress<T, P>, E>>`, or
//! `impl Stream<Item = Progress<T, P>>` when it cannot fail. Progress sits
//! *inside* the `Result` because it is on the success path: an err is never
//! "in progress".
//!
//! [`Progress`] is never serialized as a whole. The server sends the two
//! cases in different frames — [`Progress::Update`] as a progress frame
//! carrying `P`, [`Progress::Item`] as a data frame carrying `T` — and a
//! client that opts in sees them again as a [`Frame`](crate::Frame).

use serde::{Deserialize, Serialize};

/// One step of a method that reports progress: an intermediate report on the
/// way to its items, or one of the items.
///
/// `P` is the method's progress type. It defaults to [`Note`] — the shape
/// the progress frame has always carried — so `Progress<T>` is the untyped
/// case and `Progress<T, MyStep>` the precise one.
///
/// ```
/// use plexus_client::{Note, Progress};
///
/// enum ImportStep { Chunk { done: usize, of: usize } }
///
/// let step: Progress<u32, ImportStep> = Progress::Update(ImportStep::Chunk { done: 3, of: 10 });
/// let done: Progress<u32, ImportStep> = Progress::Item(42);
/// assert!(step.is_update());
/// assert_eq!(done.into_item(), Some(42));
///
/// let untyped: Progress<u32> = Progress::Update(Note::new("indexing").with_percentage(50.0));
/// assert!(untyped.is_update());
/// ```
///
/// Deliberately **not** `Serialize`/`Deserialize`: the two cases travel in
/// different frames, so there is no single wire value to encode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Progress<T, P = Note> {
    /// An intermediate report. Travels as a progress frame carrying `P`.
    Update(P),
    /// One of the method's items. Travels as a data frame carrying `T`.
    Item(T),
}

impl<T, P> Progress<T, P> {
    /// `true` for [`Progress::Update`].
    pub fn is_update(&self) -> bool {
        matches!(self, Progress::Update(_))
    }

    /// `true` for [`Progress::Item`].
    pub fn is_item(&self) -> bool {
        matches!(self, Progress::Item(_))
    }

    /// The item, if this is one; an update is dropped.
    pub fn into_item(self) -> Option<T> {
        match self {
            Progress::Item(t) => Some(t),
            Progress::Update(_) => None,
        }
    }

    /// The update, if this is one; an item is dropped.
    pub fn into_update(self) -> Option<P> {
        match self {
            Progress::Update(p) => Some(p),
            Progress::Item(_) => None,
        }
    }
}

/// The default progress type: a human-readable message and, optionally, how
/// far along the work is.
///
/// Wire shape: `{"message": "…", "percentage": 50.0}`; `percentage` is
/// omitted when the method cannot say (and absent decodes as `None`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Note {
    /// What is happening, for a human.
    pub message: String,
    /// Completion, `0.0`–`100.0`, when the method can say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub percentage: Option<f32>,
}

impl Note {
    /// A note with no percentage.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            percentage: None,
        }
    }

    /// This note, with `percentage` (`0.0`–`100.0`) set.
    pub fn with_percentage(mut self, percentage: f32) -> Self {
        self.percentage = Some(percentage);
        self
    }
}
