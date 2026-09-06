//! What a typed client hands its caller.

/// One event of a typed Plexus call.
///
/// `E` is the activation's own event enum — `TicketsEvent`, not a
/// hand-mirrored copy of it. That identity is the whole zero-drift argument:
/// rename a field on the server type and every consumer of this stream stops
/// compiling.
///
/// Terminal states are deliberately *not* variants here: normal completion
/// is the stream ending, and both error layers are the `Err` half of
/// `Result<ClientEvent<E>, ClientError>`.
#[derive(Debug, Clone, PartialEq)]
pub enum ClientEvent<E> {
    /// A payload event from the activation.
    Data(E),
    /// A progress report. Carries no payload.
    Progress {
        message: String,
        percentage: Option<f32>,
    },
}

impl<E> ClientEvent<E> {
    /// The payload, if this is a data event.
    pub fn into_data(self) -> Option<E> {
        match self {
            ClientEvent::Data(e) => Some(e),
            ClientEvent::Progress { .. } => None,
        }
    }

    /// Borrow the payload, if this is a data event.
    pub fn data(&self) -> Option<&E> {
        match self {
            ClientEvent::Data(e) => Some(e),
            ClientEvent::Progress { .. } => None,
        }
    }

    /// `true` for [`ClientEvent::Progress`].
    pub fn is_progress(&self) -> bool {
        matches!(self, ClientEvent::Progress { .. })
    }
}
