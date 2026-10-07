//! RETURNS-4 acceptance 1: every `RemoteError` variant ⇄ its wire JSON.
//!
//! The literals are the contract the Haskell protocol and the TS/Rust
//! generators mirror (RETURNS-12/13/14/17/18): `kind`-tagged, `snake_case`,
//! `ParamProblem` flattened under `problem`, `DenyReason` under `reason`.

use plexus_client::{DenyReason, ParamProblem, RemoteError};
use serde_json::{json, Value};

/// `err` serializes to exactly `wire`, and `wire` deserializes to exactly
/// `err`.
fn round_trips(err: RemoteError, wire: Value) {
    assert_eq!(
        serde_json::to_value(&err).expect("serializes"),
        wire,
        "serialize {err:?}"
    );
    assert_eq!(
        serde_json::from_value::<RemoteError>(wire.clone()).expect("deserializes"),
        err,
        "deserialize {wire}"
    );
    // And through text, the way a transport actually receives it.
    let text = serde_json::to_string(&wire).unwrap();
    assert_eq!(serde_json::from_str::<RemoteError>(&text).unwrap(), err);
}

#[test]
fn activation_not_found() {
    round_trips(
        RemoteError::ActivationNotFound {
            activation: "tickets".into(),
        },
        json!({"kind": "activation_not_found", "activation": "tickets"}),
    );
}

#[test]
fn method_not_found() {
    round_trips(
        RemoteError::MethodNotFound {
            activation: "tickets".into(),
            method: "frob".into(),
        },
        json!({"kind": "method_not_found", "activation": "tickets", "method": "frob"}),
    );
}

#[test]
fn invalid_params_missing_named_param() {
    // The literal from RETURNS-1's wire example.
    round_trips(
        RemoteError::InvalidParams {
            param: Some("id".into()),
            problem: ParamProblem::Missing,
        },
        json!({"kind": "invalid_params", "param": "id", "problem": "missing"}),
    );
}

#[test]
fn invalid_params_malformed_named_param() {
    round_trips(
        RemoteError::InvalidParams {
            param: Some("limit".into()),
            problem: ParamProblem::Malformed {
                detail: "invalid type: string \"ten\", expected u32".into(),
            },
        },
        json!({
            "kind": "invalid_params",
            "param": "limit",
            "problem": "malformed",
            "detail": "invalid type: string \"ten\", expected u32"
        }),
    );
}

#[test]
fn invalid_params_without_a_param_omits_the_key() {
    round_trips(
        RemoteError::InvalidParams {
            param: None,
            problem: ParamProblem::Malformed {
                detail: "params must be an object".into(),
            },
        },
        json!({"kind": "invalid_params", "problem": "malformed", "detail": "params must be an object"}),
    );
    round_trips(
        RemoteError::InvalidParams {
            param: None,
            problem: ParamProblem::Missing,
        },
        json!({"kind": "invalid_params", "problem": "missing"}),
    );
}

#[test]
fn invalid_params_null_param_decodes_as_none() {
    let err: RemoteError = serde_json::from_value(
        json!({"kind": "invalid_params", "param": null, "problem": "missing"}),
    )
    .unwrap();
    assert_eq!(
        err,
        RemoteError::InvalidParams {
            param: None,
            problem: ParamProblem::Missing
        }
    );
}

#[test]
fn unauthenticated() {
    round_trips(
        RemoteError::Unauthenticated,
        json!({"kind": "unauthenticated"}),
    );
}

#[test]
fn forbidden_missing_scope() {
    round_trips(
        RemoteError::Forbidden {
            reason: DenyReason::MissingScope {
                scope: "tickets:write".into(),
            },
        },
        json!({"kind": "forbidden", "reason": "missing_scope", "scope": "tickets:write"}),
    );
}

#[test]
fn forbidden_tenant_boundary() {
    round_trips(
        RemoteError::Forbidden {
            reason: DenyReason::TenantBoundary,
        },
        json!({"kind": "forbidden", "reason": "tenant_boundary"}),
    );
}

#[test]
fn forbidden_not_accepted() {
    round_trips(
        RemoteError::Forbidden {
            reason: DenyReason::NotAccepted,
        },
        json!({"kind": "forbidden", "reason": "not_accepted"}),
    );
}

#[test]
fn handle_not_supported() {
    round_trips(
        RemoteError::HandleNotSupported {
            activation: "tickets".into(),
        },
        json!({"kind": "handle_not_supported", "activation": "tickets"}),
    );
}

#[test]
fn execution() {
    round_trips(
        RemoteError::Execution {
            message: "database is locked".into(),
        },
        json!({"kind": "execution", "message": "database is locked"}),
    );
}

/// Exhaustive means no catch-all: a kind this build does not know is a
/// decode error, not a silently-swallowed `Unknown`.
#[test]
fn unknown_kind_is_a_decode_error() {
    let e = serde_json::from_value::<RemoteError>(json!({"kind": "rate_limited"}))
        .expect_err("no catch-all variant");
    assert!(e.to_string().contains("rate_limited"), "{e}");

    let e =
        serde_json::from_value::<RemoteError>(json!({"kind": "forbidden", "reason": "too_tall"}))
            .expect_err("no catch-all deny reason");
    assert!(e.to_string().contains("too_tall"), "{e}");

    let e = serde_json::from_value::<RemoteError>(
        json!({"kind": "invalid_params", "param": "id", "problem": "smelly"}),
    )
    .expect_err("no catch-all param problem");
    assert!(e.to_string().contains("smelly"), "{e}");
}

/// A required field that is absent is a decode error.
#[test]
fn missing_fields_are_decode_errors() {
    for wire in [
        json!({"kind": "method_not_found", "activation": "tickets"}),
        json!({"kind": "invalid_params", "param": "id"}),
        json!({"kind": "invalid_params", "problem": "malformed"}),
        json!({"kind": "forbidden"}),
        json!({"kind": "forbidden", "reason": "missing_scope"}),
        json!({"activation": "tickets"}),
    ] {
        assert!(
            serde_json::from_value::<RemoteError>(wire.clone()).is_err(),
            "{wire} must not decode"
        );
    }
}

/// `Display` is one human-readable line — what a CLI prints — and names the
/// facts the variant carries (and, for `forbidden`, only those).
#[test]
fn display_is_one_line() {
    let cases = [
        (
            RemoteError::ActivationNotFound {
                activation: "tickets".into(),
            },
            "activation not found: tickets",
        ),
        (
            RemoteError::MethodNotFound {
                activation: "tickets".into(),
                method: "frob".into(),
            },
            "method not found: tickets.frob",
        ),
        (
            RemoteError::InvalidParams {
                param: Some("id".into()),
                problem: ParamProblem::Missing,
            },
            "invalid params: missing `id`",
        ),
        (
            RemoteError::InvalidParams {
                param: None,
                problem: ParamProblem::Missing,
            },
            "invalid params: a required parameter is missing",
        ),
        (
            RemoteError::InvalidParams {
                param: Some("limit".into()),
                problem: ParamProblem::Malformed {
                    detail: "expected u32".into(),
                },
            },
            "invalid params: malformed `limit`: expected u32",
        ),
        (
            RemoteError::InvalidParams {
                param: None,
                problem: ParamProblem::Malformed {
                    detail: "params must be an object".into(),
                },
            },
            "invalid params: malformed: params must be an object",
        ),
        (
            RemoteError::Unauthenticated,
            "unauthenticated: credentials required",
        ),
        (
            RemoteError::Forbidden {
                reason: DenyReason::MissingScope {
                    scope: "tickets:write".into(),
                },
            },
            "forbidden: missing required scope `tickets:write`",
        ),
        (
            RemoteError::Forbidden {
                reason: DenyReason::TenantBoundary,
            },
            "forbidden: tenant boundary",
        ),
        (
            RemoteError::Forbidden {
                reason: DenyReason::NotAccepted,
            },
            "forbidden: call not accepted",
        ),
        (
            RemoteError::HandleNotSupported {
                activation: "tickets".into(),
            },
            "handle resolution not supported by activation: tickets",
        ),
        (
            RemoteError::Execution {
                message: "database is locked".into(),
            },
            "execution failed: database is locked",
        ),
    ];
    for (err, line) in cases {
        assert_eq!(err.to_string(), line);
    }
}

/// Free text from the server cannot break the one line.
#[test]
fn display_folds_line_breaks_in_free_text() {
    let err = RemoteError::Execution {
        message: "panicked at src/lib.rs:1:1:\nboom\r\nagain".into(),
    };
    let line = err.to_string();
    assert!(!line.contains('\n') && !line.contains('\r'), "{line:?}");
    assert_eq!(
        line,
        "execution failed: panicked at src/lib.rs:1:1: boom  again"
    );
}

#[test]
fn is_a_std_error() {
    fn takes(_: &dyn std::error::Error) {}
    takes(&RemoteError::Unauthenticated);
    let boxed: Box<dyn std::error::Error + Send + Sync> = Box::new(RemoteError::Unauthenticated);
    assert_eq!(boxed.to_string(), "unauthenticated: credentials required");
}
