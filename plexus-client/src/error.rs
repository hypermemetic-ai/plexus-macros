//! The two error layers a Plexus client sees, kept apart by type.
//!
//! The wire has two independent failure modes and conflating them is the
//! bug this split exists to prevent:
//!
//! | Layer | Meaning | Type |
//! |---|---|---|
//! | transport | the connection/HTTP call failed; nothing was said about the request | [`TransportError`] |
//! | in-band | the server answered, and the answer is an `error` item | [`ClientError::Remote`] |
//!
//! `menger/web/index.html` distinguishes these by depth
//! (`item.type === "error"` vs `item.content.type === "error"`); here the
//! distinction is in the type system instead.

use std::fmt;

/// What kind of transport failure occurred.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TransportErrorKind {
    /// Could not establish the connection at all.
    Connect,
    /// The peer spoke something this transport could not parse (a malformed
    /// JSON-RPC frame, an SSE frame with no `data:` line that was required,
    /// a JSON-RPC `error` response to the subscribe call).
    Protocol,
    /// A non-success HTTP status.
    Http { status: u16 },
    /// The stream ended without a terminal item.
    Closed,
    /// Anything the transport wants to report that is none of the above.
    Other,
}

/// A failure of the transport itself — the request never got an answer, or
/// the answer was unintelligible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportError {
    pub kind: TransportErrorKind,
    pub message: String,
}

impl TransportError {
    pub fn new(kind: TransportErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
    pub fn connect(message: impl Into<String>) -> Self {
        Self::new(TransportErrorKind::Connect, message)
    }
    pub fn protocol(message: impl Into<String>) -> Self {
        Self::new(TransportErrorKind::Protocol, message)
    }
    pub fn http(status: u16, message: impl Into<String>) -> Self {
        Self::new(TransportErrorKind::Http { status }, message)
    }
    pub fn closed(message: impl Into<String>) -> Self {
        Self::new(TransportErrorKind::Closed, message)
    }
    pub fn other(message: impl Into<String>) -> Self {
        Self::new(TransportErrorKind::Other, message)
    }
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            TransportErrorKind::Connect => write!(f, "transport connect: {}", self.message),
            TransportErrorKind::Protocol => write!(f, "transport protocol: {}", self.message),
            TransportErrorKind::Http { status } => {
                write!(f, "transport http {}: {}", status, self.message)
            }
            TransportErrorKind::Closed => write!(f, "transport closed: {}", self.message),
            TransportErrorKind::Other => write!(f, "transport: {}", self.message),
        }
    }
}

impl std::error::Error for TransportError {}

/// Everything that can go wrong on a typed client call.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum ClientError {
    /// The transport failed. See [`TransportError`].
    Transport(TransportError),

    /// The server answered with an in-band `error` item. This is the
    /// activation or the hub saying no — the transport worked fine.
    Remote {
        message: String,
        code: Option<String>,
        recoverable: bool,
    },

    /// A `data` item arrived whose `content` did not deserialize into the
    /// method's event type. This is a contract break between client and
    /// server (a renamed field, a version skew) and terminates the stream.
    Decode {
        /// The item's `content_type`, when the transport carried one.
        content_type: Option<String>,
        message: String,
    },

    /// An argument could not be serialized into the call's params object,
    /// so nothing was sent. A generated client reports this rather than
    /// panicking — a panic in a wasm32 consumer is an unrecoverable trap.
    Encode {
        /// The parameter that failed.
        param: &'static str,
        message: String,
    },
}

impl ClientError {
    /// `true` when the failure came from the transport rather than the peer.
    pub fn is_transport(&self) -> bool {
        matches!(self, ClientError::Transport(_))
    }
    /// `true` when the peer answered with an in-band `error` item.
    pub fn is_remote(&self) -> bool {
        matches!(self, ClientError::Remote { .. })
    }
}

impl From<TransportError> for ClientError {
    fn from(e: TransportError) -> Self {
        ClientError::Transport(e)
    }
}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClientError::Transport(e) => write!(f, "{e}"),
            ClientError::Remote {
                message,
                code: Some(code),
                ..
            } => write!(f, "remote error [{code}]: {message}"),
            ClientError::Remote { message, .. } => write!(f, "remote error: {message}"),
            ClientError::Decode {
                content_type: Some(ct),
                message,
            } => write!(f, "decode {ct}: {message}"),
            ClientError::Decode { message, .. } => write!(f, "decode: {message}"),
            ClientError::Encode { param, message } => {
                write!(f, "encode param `{param}`: {message}")
            }
        }
    }
}

impl std::error::Error for ClientError {}
