//! LOCAL-3 (spike, never merged): a connection is symmetric — the surface a
//! peer **offers** on a connection it opened, for the other side to call.
//!
//! A client that dialed a service may offer one namespace of its own
//! activations on that socket ([`crate::WsTransport::offer`]). The accepting
//! service then opens calls into it with the very frames a client sends a
//! service — `{ns}.call {method, params, hop?}`, a subscription id, items,
//! `{ns}.call_unsub` — and the dialer's own [`Offered`] answers them: its own
//! gate, as the peer, for whom the peer acts.
//!
//! This module is the wasm-clean half: the trait an offered surface
//! implements and what one call into it carries. The runtime that serves it
//! on a socket is `ws::served` (`ws-native`).

use futures_util::stream::BoxStream;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// `hop` on a call (plexus-wire spec/12): who acts, for whom, within what.
///
/// On a call into a peer, `principal` is informative only: the callee takes
/// the caller's principal from the connection (what the peer proved, or what
/// the dialer pinned for the address it dialed), never from the frame. `for`
/// names whom the caller acts for and grants nothing.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Hop {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub principal: Option<String>,
    #[serde(rename = "for", default)]
    pub for_: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call: Option<String>,
}

/// One call into an offered surface.
pub struct ServedCall {
    /// The dotted method, `ci.run`.
    pub method: String,
    pub params: Value,
    /// The frame's `hop`, if it carried one.
    pub hop: Option<Hop>,
    /// The call's input half, in `seq` order, when the method takes one
    /// ([`Offered::has_input`]); `None` otherwise.
    pub input: Option<BoxStream<'static, Value>>,
}

impl std::fmt::Debug for ServedCall {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServedCall")
            .field("method", &self.method)
            .field("hop", &self.hop)
            .field("input", &self.input.is_some())
            .finish()
    }
}

/// A surface a peer offers on its connection.
pub trait Offered: Send + Sync + 'static {
    /// The namespace the calls name (`{ns}.call`) — the dialer's own.
    fn namespace(&self) -> &str;
    /// The activations offered; a call to any other is refused `not_offered`
    /// before anything runs.
    fn activations(&self) -> Vec<String>;
    /// The offered activations' schema, sent with the offer.
    fn schema(&self) -> Value {
        Value::Null
    }
    /// Whether `method` takes an input half (BIDIR-6 reads it off the
    /// schema; the spike asks).
    fn has_input(&self, _method: &str) -> bool {
        false
    }
    /// Run one call: wire items (`{type: data|progress|error|done, …}`), the
    /// last one terminal.
    fn serve(&self, call: ServedCall) -> BoxStream<'static, Value>;
}
