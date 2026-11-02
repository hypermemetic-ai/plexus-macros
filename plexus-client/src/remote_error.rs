//! The typed, exhaustive vocabulary of call failures.
//!
//! A call fails — as opposed to an activation saying no, which is an `err`
//! item and therefore data — when the hub could not or would not run it:
//! no such activation or method, bad params, no credentials, not allowed,
//! or the method itself blew up. The error frame carries exactly one
//! [`RemoteError`], and so does the `data` of a JSON-RPC-level rejection.
//!
//! # Wire shape
//!
//! `kind`-tagged, `snake_case`. [`ParamProblem`] and [`DenyReason`] are
//! flattened into their variant, each under the key its field is named for:
//!
//! ```json
//! {"kind": "activation_not_found", "activation": "tickets"}
//! {"kind": "method_not_found", "activation": "tickets", "method": "frob"}
//! {"kind": "invalid_params", "param": "id", "problem": "missing"}
//! {"kind": "invalid_params", "param": "id", "problem": "malformed", "detail": "expected a string"}
//! {"kind": "unauthenticated"}
//! {"kind": "forbidden", "reason": "missing_scope", "scope": "tickets:write"}
//! {"kind": "forbidden", "reason": "tenant_boundary"}
//! {"kind": "handle_not_supported", "activation": "tickets"}
//! {"kind": "execution", "message": "database is locked"}
//! {"kind": "forbidden", "reason": "missing_grant", "grant": "tickets.create", "scope": "*", "roles": ["editor", "admin"], "held_on": []}
//! {"kind": "too_many_calls", "max": 8}
//! {"kind": "child_unavailable", "child": "echo", "reason": "the connection has closed"}
//! {"kind": "child_timeout", "child": "echo", "timeout_ms": 5000}
//! ```
//!
//! The last four are main's refusals (AUTH-2's typed refusal, spec/05's
//! `too_many_calls`, spec/12's `child_unavailable` and `child_timeout`),
//! brought into this vocabulary by RETURNS-36: every error frame a server
//! sends carries one `RemoteError`, beside spec/05's flattened
//! `code` / `message` / `error_kind` fields.
//!
//! `param` is omitted when the failure is not about one parameter (and
//! decodes as `None` when absent or `null`).
//!
//! # Exhaustive on purpose
//!
//! No `#[non_exhaustive]`, no catch-all variant: a new failure kind must
//! fail every receiver's build until it handles it. Adding a variant here
//! means adding its `PlexusError` conversion in plexus-core and its mirrors
//! in the Haskell protocol and the TS/Rust generators in the same change.

use std::fmt::{self, Write as _};

use serde::{Deserialize, Serialize};

/// Why a call failed — what an error frame carries, and the `data` of a
/// JSON-RPC-level rejection.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RemoteError {
    /// The hub has no activation by this name.
    ActivationNotFound { activation: String },

    /// The activation exists but has no method by this name.
    MethodNotFound { activation: String, method: String },

    /// The call's params did not fit the method.
    InvalidParams {
        /// The parameter at fault, when the failure is about one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        param: Option<String>,
        /// What is wrong with it. Flattened: its `problem` tag sits beside
        /// `param` on the wire.
        #[serde(flatten)]
        problem: ParamProblem,
    },

    /// The method requires credentials and the call carried none that
    /// verified.
    Unauthenticated,

    /// The caller is authenticated but not allowed this call.
    Forbidden {
        /// Which check the caller failed. Flattened: its `reason` tag sits
        /// beside `kind` on the wire.
        #[serde(flatten)]
        reason: DenyReason,
    },

    /// The activation does not resolve handles.
    HandleNotSupported { activation: String },

    /// The method ran and failed for a reason that is not the activation
    /// saying no (that is an `err` item): a panic, a lost dependency.
    Execution { message: String },

    /// spec/05, 09: the connection — or the child the call was routed to —
    /// holds its maximum of open calls; close one and call again.
    TooManyCalls {
        /// The most calls it holds open at once.
        max: u64,
        /// The child whose limit it is, when the limit is a child's.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        child: Option<String>,
    },

    /// spec/12 rule 5: the child the call was routed to is not there.
    ChildUnavailable { child: String, reason: String },

    /// The child the call was routed to did not open it in the service's
    /// time.
    ChildTimeout { child: String, timeout_ms: u64 },
}

/// What is wrong with a call's params. Flattened into
/// [`RemoteError::InvalidParams`] under the `problem` key.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(tag = "problem", rename_all = "snake_case")]
pub enum ParamProblem {
    /// A required parameter was not supplied.
    Missing,
    /// A parameter was supplied but did not decode; `detail` says how.
    Malformed { detail: String },
}

/// Why an authenticated caller was refused. Flattened into
/// [`RemoteError::Forbidden`] under the `reason` key.
///
/// Each variant carries at most the single fact the caller already failed
/// (AUTHZ-CORE-1/5, rendered per AUTHZ-PRIVACY-4): the first unmet scope,
/// never the method's full requirement set or the registry's taxonomy.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum DenyReason {
    /// The caller's scopes do not include `scope` — the first unmet one.
    MissingScope { scope: String },
    /// The call reaches across the caller's tenant boundary.
    TenantBoundary,
    /// The call was not accepted in the caller's action context.
    NotAccepted,
    /// AUTH-2: the grant gate's refusal — the caller does not hold `grant`
    /// on `scope`. Names what was needed and where the caller does hold it
    /// (owner, 2026-10-10: "return the grants required to hit an
    /// endpoint").
    MissingGrant {
        /// The method's grant, `<activation>.<method>`.
        grant: String,
        /// Where it was needed: the target's innermost scope, or `*`.
        scope: String,
        /// The roles that list the grant, `admin` last.
        #[serde(default)]
        roles: Vec<String>,
        /// Where the caller holds it.
        #[serde(default)]
        held_on: Vec<String>,
    },
}

impl RemoteError {
    /// The variant's `kind` tag, as on the wire (`"method_not_found"`, …) —
    /// for a log line or an exit code; match on the variant to handle it.
    pub fn kind(&self) -> &'static str {
        match self {
            RemoteError::ActivationNotFound { .. } => "activation_not_found",
            RemoteError::MethodNotFound { .. } => "method_not_found",
            RemoteError::InvalidParams { .. } => "invalid_params",
            RemoteError::Unauthenticated => "unauthenticated",
            RemoteError::Forbidden { .. } => "forbidden",
            RemoteError::HandleNotSupported { .. } => "handle_not_supported",
            RemoteError::Execution { .. } => "execution",
            RemoteError::TooManyCalls { .. } => "too_many_calls",
            RemoteError::ChildUnavailable { .. } => "child_unavailable",
            RemoteError::ChildTimeout { .. } => "child_timeout",
        }
    }
}

/// Writes `s` with every line break folded to a space, so the line a CLI
/// prints stays one line whatever the server put in a free-text field.
fn one_line(f: &mut fmt::Formatter<'_>, s: &str) -> fmt::Result {
    for c in s.chars() {
        f.write_char(if matches!(c, '\n' | '\r') { ' ' } else { c })?;
    }
    Ok(())
}

impl fmt::Display for RemoteError {
    /// One line, for a human — what a CLI prints.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RemoteError::ActivationNotFound { activation } => {
                f.write_str("activation not found: ")?;
                one_line(f, activation)
            }
            RemoteError::MethodNotFound { activation, method } => {
                f.write_str("method not found: ")?;
                one_line(f, activation)?;
                f.write_char('.')?;
                one_line(f, method)
            }
            RemoteError::InvalidParams { param, problem } => {
                f.write_str("invalid params: ")?;
                match (problem, param) {
                    (ParamProblem::Missing, Some(p)) => {
                        f.write_str("missing `")?;
                        one_line(f, p)?;
                        f.write_char('`')
                    }
                    (ParamProblem::Missing, None) => f.write_str("a required parameter is missing"),
                    (ParamProblem::Malformed { detail }, Some(p)) => {
                        f.write_str("malformed `")?;
                        one_line(f, p)?;
                        f.write_str("`: ")?;
                        one_line(f, detail)
                    }
                    (ParamProblem::Malformed { detail }, None) => {
                        f.write_str("malformed: ")?;
                        one_line(f, detail)
                    }
                }
            }
            RemoteError::Unauthenticated => f.write_str("unauthenticated: credentials required"),
            RemoteError::Forbidden { reason } => {
                f.write_str("forbidden: ")?;
                match reason {
                    DenyReason::MissingScope { scope } => {
                        f.write_str("missing required scope `")?;
                        one_line(f, scope)?;
                        f.write_char('`')
                    }
                    DenyReason::TenantBoundary => f.write_str("tenant boundary"),
                    DenyReason::NotAccepted => f.write_str("call not accepted"),
                    DenyReason::MissingGrant { grant, scope, .. } => {
                        f.write_str("needs `")?;
                        one_line(f, grant)?;
                        f.write_char('`')?;
                        if scope != "*" {
                            f.write_str(" on `")?;
                            one_line(f, scope)?;
                            f.write_char('`')?;
                        }
                        Ok(())
                    }
                }
            }
            RemoteError::HandleNotSupported { activation } => {
                f.write_str("handle resolution not supported by activation: ")?;
                one_line(f, activation)
            }
            RemoteError::Execution { message } => {
                f.write_str("execution failed: ")?;
                one_line(f, message)
            }
            RemoteError::TooManyCalls { max, child } => {
                f.write_str("too many calls open")?;
                if let Some(child) = child {
                    f.write_str(" on child ")?;
                    one_line(f, child)?;
                }
                write!(f, " (at most {max}): close one, or call again later")
            }
            RemoteError::ChildUnavailable { child, reason } => {
                f.write_str("child ")?;
                one_line(f, child)?;
                f.write_str(" is unavailable: ")?;
                one_line(f, reason)
            }
            RemoteError::ChildTimeout { child, timeout_ms } => {
                f.write_str("child ")?;
                one_line(f, child)?;
                write!(f, " did not open the call in {timeout_ms} ms")
            }
        }
    }
}

impl std::error::Error for RemoteError {}
