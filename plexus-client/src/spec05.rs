//! RETURNS-36 · a peer on plexus-wire's spec as main wrote it (spec/03, 05),
//! read into 0.8's shapes — one rule for the client and plexus-core's child
//! seam (its `remote_child::peer`), never guessed from anything the peer did
//! not say:
//!
//! - a progress frame without `content` is the default progress type, a
//!   [`Note`](crate::Note): `{message, percentage}`;
//! - an error frame without `error` is typed from its spec/05 fields
//!   ([`remote_error`]): by `error_kind`, else by `code` — `-32601` not
//!   found, `-32602` invalid params, `-32001` unauthenticated, `-32003`
//!   forbidden — a kind this build does not know kept whole
//!   ([`RemoteError::Other`]), and nothing named the method failing.
//!
//! A 0.8 frame is never rewritten: each rule fills only what the peer left
//! out, and a reader asks only when a frame did not read as 0.8.

use serde_json::{Map, Value};

use crate::remote_error::{DenyReason, OtherError, ParamProblem, RemoteError};

/// spec/05's error fields as the typed [`RemoteError`] they say.
pub fn remote_error(o: &Map<String, Value>) -> RemoteError {
    let s = |k: &str| o.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    let strings = |k: &str| -> Vec<String> {
        o.get(k).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default()
    };
    let message = s("message");
    let code = match o.get("code") {
        Some(Value::String(c)) => c.clone(),
        Some(other) => other.to_string(),
        None => String::new(),
    };
    match o.get("error_kind").and_then(Value::as_str).unwrap_or(code.as_str()) {
        // spec/05: activation or method not found (review #5).
        "-32601" => not_found(&message),
        "invalid_params" | "-32602" => RemoteError::InvalidParams { param: None, problem: ParamProblem::Malformed { detail: message } },
        "unauthenticated" | "-32001" => RemoteError::Unauthenticated,
        "forbidden" | "-32003" => RemoteError::Forbidden {
            reason: match o.get("reason").and_then(Value::as_str) {
                Some("missing_scope") => DenyReason::MissingScope { scope: s("scope") },
                Some("tenant_boundary") => DenyReason::TenantBoundary,
                Some("not_accepted") => DenyReason::NotAccepted,
                Some("not_needed") => DenyReason::NotNeeded { method: s("method") },
                _ => DenyReason::MissingGrant { grant: s("grant"), scope: s("scope"), roles: strings("roles"), held_on: strings("held_on") },
            },
        },
        "too_many_calls" => RemoteError::TooManyCalls {
            max: o.get("max").and_then(Value::as_u64).unwrap_or_default(),
            child: o.get("child").and_then(Value::as_str).map(str::to_string),
        },
        "child_unavailable" => RemoteError::ChildUnavailable { child: s("child"), reason: s("reason") },
        "child_timeout" => RemoteError::ChildTimeout {
            child: s("child"),
            timeout_ms: o.get("timeout_ms").and_then(Value::as_u64).unwrap_or_default(),
        },
        // A kind this build does not know, by its `error_kind`, keeps it and
        // its fields (review #4); with no kind at all, the method failing.
        other if o.contains_key("error_kind") => {
            let mut fields = o.clone();
            fields.remove("error_kind");
            for k in ["type", "metadata", "code", "recoverable"] {
                fields.remove(k);
            }
            RemoteError::Other(OtherError { kind: other.to_string(), fields })
        }
        _ => RemoteError::Execution { message },
    }
}

/// `-32601`'s two meanings, from spec/05's message (`Activation not found:
/// x`, `Method not found: a.b`); a bare one is a method not found.
fn not_found(message: &str) -> RemoteError {
    if let Some(activation) = message.strip_prefix("Activation not found: ") {
        return RemoteError::ActivationNotFound { activation: activation.trim().to_string() };
    }
    let path = message.strip_prefix("Method not found: ").unwrap_or_default().trim();
    let (activation, method) = path.rsplit_once('.').unwrap_or(("", path));
    RemoteError::MethodNotFound { activation: activation.to_string(), method: method.to_string() }
}

/// A stream item that did not read as 0.8, in 0.8's shape: a progress frame
/// without `content`, an error frame without `error`. `None` for anything
/// else (it stays unreadable, as it was).
pub fn item(v: &Value) -> Option<Value> {
    let mut v = v.clone();
    let o = v.as_object_mut()?;
    match o.get("type").and_then(Value::as_str)? {
        "progress" if !o.contains_key("content") => {
            let mut note = Map::new();
            note.insert("message".into(), o.remove("message").unwrap_or(Value::from("")));
            if let Some(p) = o.remove("percentage").filter(|p| !p.is_null()) {
                note.insert("percentage".into(), p);
            }
            o.insert("content".into(), Value::Object(note));
        }
        "error" if !o.contains_key("error") => {
            let error = remote_error(o);
            o.insert("error".into(), serde_json::to_value(error).ok()?);
        }
        _ => return None,
    }
    Some(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::envelope::StreamItem;
    use serde_json::json;

    #[test]
    fn a_main_wire_error_and_progress_read_as_0_8() {
        let e = item(&json!({"type": "error", "message": "needs a.b", "code": "-32003", "error_kind": "forbidden",
                             "grant": "a.b", "scope": "*", "roles": ["admin"], "held_on": [], "recoverable": false}))
        .unwrap();
        let StreamItem::Error { error, .. } = serde_json::from_value(e).unwrap() else { panic!() };
        assert_eq!(error.kind(), "forbidden");
        let p = item(&json!({"type": "progress", "message": "half", "percentage": 50.0})).unwrap();
        assert_eq!(p["content"], json!({"message": "half", "percentage": 50.0}));
        assert!(serde_json::from_value::<StreamItem>(p).is_ok());
    }

    #[test]
    fn a_0_8_frame_is_never_rewritten() {
        assert!(item(&json!({"type": "error", "error": {"kind": "unauthenticated"}})).is_none());
        assert!(item(&json!({"type": "progress", "content": 1})).is_none());
        assert!(item(&json!({"type": "data", "content": 1})).is_none());
    }
}
