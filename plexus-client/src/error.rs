//! The two error layers a Plexus client sees, kept apart by type.
//!
//! The wire has two independent failure modes and conflating them is the
//! bug this split exists to prevent:
//!
//! | Layer | Meaning | Type |
//! |---|---|---|
//! | transport | the connection/HTTP call failed; nothing was said about the request | [`TransportError`] |
//! | remote | the server answered, and the answer is an error frame: the call failed, typed | [`ClientError::Remote`] ([`RemoteError`]) |
//!
//! Neither is the activation saying no. That is an `err` *item* — data, the
//! `Err` half of a `Result<T, E>` item — and it reaches the caller as a
//! value, never as a `ClientError` (RETURNS-1 decision 8). The one rule for
//! a receiver: `?` is the call failing; a match on the item is the
//! activation's answer.
//!
//! ```
//! # use plexus_client::{ClientError, RemoteError};
//! # fn show(_: u32) {}
//! # fn missing(_: &str) {}
//! # fn check(answer: Result<Result<u32, String>, ClientError>) -> Result<(), ClientError> {
//! match answer? {                       // ? = the call failed
//!     Ok(n) => show(n),                 // the activation's yes
//!     Err(why) => missing(&why),        // the activation's no
//! }
//! # Ok(()) }
//! # let failed: Result<Result<u32, String>, ClientError> = Err(ClientError::Remote(RemoteError::Unauthenticated));
//! # assert!(check(failed).unwrap_err().is_remote());
//! ```

use std::fmt;

use crate::remote_error::RemoteError;

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

/// Everything that can go wrong on a typed client call — the call failing,
/// never the activation saying no (that is an `err` item).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum ClientError {
    /// The transport failed. See [`TransportError`].
    Transport(TransportError),

    /// The server answered with an error frame: the call failed — no such
    /// activation or method, bad params, not authenticated, not allowed, or
    /// the method blew up. Typed and exhaustive; match on it.
    ///
    /// The frame's `recoverable` flag is not carried (RETURNS-1 decision
    /// 9): an error frame ends the call either way.
    Remote(RemoteError),

    /// The server's answer does not fit the method's declared contract: a
    /// `data` item whose `content` does not deserialize into the method's
    /// item type, a `progress` item that does not deserialize into its
    /// progress type, or a one-item method that sent no item or more than
    /// one. A contract break between client and server (a renamed field, a
    /// version skew); it terminates the stream.
    Decode {
        /// The item's `content_type` (`"tickets.get"`), when the transport
        /// carried one; the called method's, when the break is in the count.
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

    /// `true` when the peer answered with an error frame.
    pub fn is_remote(&self) -> bool {
        matches!(self, ClientError::Remote(_))
    }

    /// The error frame's [`RemoteError`], when the call failed remotely.
    pub fn remote(&self) -> Option<&RemoteError> {
        match self {
            ClientError::Remote(e) => Some(e),
            _ => None,
        }
    }
}

impl From<TransportError> for ClientError {
    fn from(e: TransportError) -> Self {
        ClientError::Transport(e)
    }
}

impl From<RemoteError> for ClientError {
    fn from(e: RemoteError) -> Self {
        ClientError::Remote(e)
    }
}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClientError::Transport(e) => write!(f, "{e}"),
            ClientError::Remote(e) => write!(f, "remote error: {e}"),
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

impl std::error::Error for ClientError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ClientError::Transport(e) => Some(e),
            ClientError::Remote(e) => Some(e),
            ClientError::Decode { .. } | ClientError::Encode { .. } => None,
        }
    }
}
