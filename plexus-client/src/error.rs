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

    /// RETURNS-46: a dynamic child's instance name the service would route
    /// elsewhere than to that instance — so nothing was sent. Every call
    /// through the instance's client, and through any client below it, is
    /// this, at once.
    InvalidName {
        /// The name the accessor was given.
        name: String,
        /// Why it cannot name an instance.
        reason: NameRefusal,
    },

    /// NEST-14: an untyped client made typed (`Untyped::typed`) against a
    /// node that does not serve the typed client's interface — the methods
    /// it serves under another shape, or not at all. Nothing was called.
    ShapeMismatch {
        /// The node's dotted path.
        path: String,
        /// The typed client's interface hash.
        wanted: String,
        /// The node's.
        got: String,
        /// The client's methods the node serves otherwise, or not at all.
        methods: Vec<String>,
    },
}

/// Why an instance name cannot reach its instance (RETURNS-46): the service
/// routes `<parent>.<name>.<method>` by `name`'s segment — a static child
/// before the dynamic one — and refuses a path whose segments are not
/// `[a-z][a-z0-9_]*` (plexus-auth-core's `MethodPath`). A name that is a
/// method of the parent is the instance: the service takes the first segment
/// after the parent as a child, never as the parent's method.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum NameRefusal {
    /// `""`: there is no segment.
    Empty,
    /// It holds a `.`: the service would read two segments.
    Dotted,
    /// It is not a segment of the service's path grammar,
    /// `[a-z][a-z0-9_]*`: the service refuses the call (`" "`, `"LEFT"`,
    /// `"acme-co"`, `"ａcme"`).
    NotASegment,
    /// It is a static sibling's segment: the service routes it there.
    StaticChild,
}

impl fmt::Display for NameRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            NameRefusal::Empty => "it is empty",
            NameRefusal::Dotted => "it contains '.', which the service reads as two segments",
            NameRefusal::StaticChild => "it is a static child's segment, which the service routes there",
            NameRefusal::NotASegment => "it is not a route segment ([a-z][a-z0-9_]*), which the service refuses",
        })
    }
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
            // Main's line: the method's own failure reads as the message it
            // carries ("remote error: no"); any other kind as its line.
            ClientError::Remote(crate::RemoteError::Execution { message }) => write!(f, "remote error: {message}"),
            ClientError::Remote(e) => write!(f, "remote error: {e}"),
            ClientError::Decode {
                content_type: Some(ct),
                message,
            } => write!(f, "decode {ct}: {message}"),
            ClientError::Decode { message, .. } => write!(f, "decode: {message}"),
            ClientError::Encode { param, message } => {
                write!(f, "encode param `{param}`: {message}")
            }
            ClientError::InvalidName { name, reason } => {
                write!(f, "instance name {name:?} refused, nothing sent: {reason}")
            }
            ClientError::ShapeMismatch { path, wanted, got, methods } => write!(
                f,
                "`{path}` does not serve the interface wanted ({wanted}, it serves {got}): {}",
                methods.join(", ")
            ),
        }
    }
}

impl std::error::Error for ClientError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ClientError::Transport(e) => Some(e),
            ClientError::Remote(e) => Some(e),
            ClientError::Decode { .. }
            | ClientError::Encode { .. }
            | ClientError::InvalidName { .. }
            | ClientError::ShapeMismatch { .. } => None,
        }
    }
}
