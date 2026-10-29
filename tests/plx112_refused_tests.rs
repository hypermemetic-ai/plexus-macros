//! PLX-112 — a considered "no" has a spelling, and it lands as `Refused`.
//!
//! # What was missing
//!
//! RFC 002 §6.6 makes `Refused` a distinct `StopKind` precisely because a
//! refusal is not a failure. PLX-110 landed the unary `Err` arm with
//! `E: Into<TurnError>` and, in doing so, *defined* `Err` as `Failed`:
//! `TurnError::into_stop_reason` produces `StopReason::failed` and nothing
//! else. That was never the §6.6 conflation the RFC warns about — nothing
//! mapped a refusal *onto* `Failed` — but a considered "no" had no way to be
//! said at all, by a unary method or by the streaming arm.
//!
//! # THE MECHANISM, named (acceptance criterion c1)
//!
//! **`plexus_core::runtime::IntoTurnStop`, with a blanket impl over every
//! `E: Into<TurnError>` that yields `TurnStop::Failed`.**
//!
//! - A generated handler's `Err` arm emits one fully-qualified trait call:
//!   `TurnStop::into_handler_result(IntoTurnStop::into_turn_stop(__e))`.
//!   **The macro guesses nothing** — it names no kind, inspects no type and
//!   reads no attribute. The author's own impl decides. That is the same bar
//!   PLX-110 set for `Into<TurnError>`, one level up.
//! - **`Failed` stays the default.** The blanket impl means every error type
//!   that only says `impl From<MyError> for TurnError` behaves exactly as it did
//!   before this build, and `E = TurnError` still needs no impl at all. See
//!   `an_error_that_says_nothing_about_kind_still_terminates_failed`.
//! - Writing *both* impls for one type is a coherence conflict (E0119) that
//!   names both impls, not a silent precedence rule.
//! - `String` still satisfies neither side, so PLX-110's closed door on the
//!   flattened `ExecutionError(String)` counterfactual stays closed.
//!
//! # Why not a `kind` field on `TurnError` (the rejected alternative)
//!
//! RFC 002 §6.7.1: "a terminal whose kind is not **Failed** MUST NOT carry a
//! structured error […] a **Refused** terminal with an error is the specific
//! case a checker MUST reject." A `TurnError` that could be `Refused` is that
//! rejected value with a discriminant bolted on, and it would have put the kind
//! inside the envelope whose wire shape §3.6.1 had just been amended (PLX-114)
//! to fix. Routing the non-error kinds onto the `Ok` half as `TurnOutcome`
//! — which is where `plexus-core` had already put `Refused` and `Limited`,
//! unreachable from generated code until now — keeps §6.7.1 true *by
//! construction*: only the `Err` half is ever rendered through
//! `TurnError::into_stop_reason`.
//!
//! # `Cancelled` and `Limited` (acceptance criterion c3)
//!
//! - **`Limited` is IN SCOPE and reachable**, as `TurnStop::Limited`. It is the
//!   same shape as `Refused` (open detail, no error), `TurnOutcome::Limited`
//!   already existed, and §6.5 names it. See
//!   `a_bound_lands_as_limited_not_failed`.
//! - **`Cancelled` is DEFERRED, deliberately.** RFC 002 §6.8 makes cancellation
//!   *cooperative*: a cancel signal MUST be delivered to the turn and the turn
//!   MUST resolve `Cancelled`. An `Err` value that could claim `Cancelled` would
//!   let a turn assert a signal that was never delivered, making the kind
//!   unfalsifiable for every consumer. `Cancelled` stays the runtime's to emit
//!   — the same call `plexus-core` had already made when `TurnOutcome` was given
//!   `Refused` and `Limited` variants and no `Cancelled` one. A handler with a
//!   genuine need has `TurnOutcome::Stopped` on the success side.

use futures::StreamExt;
use plexus_core::ir::{StopDetail, StopKind};
use plexus_core::plexus::{Activation, PlexusStreamItem};
use plexus_core::runtime::{IntoTurnStop, TurnError, TurnStop};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Fixture
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
struct Answer {
    v: u32,
}

/// A considered "no". It implements `IntoTurnStop` and deliberately does NOT
/// implement `Into<TurnError>` — the two are mutually exclusive by coherence.
#[derive(Debug)]
struct PermissionDenied {
    tool: String,
}

impl IntoTurnStop for PermissionDenied {
    fn into_turn_stop(self) -> TurnStop {
        TurnStop::Refused(
            StopDetail::new("acp:permission_denied")
                .with_message("the user declined the permission request")
                .with_data("tool", serde_json::json!(self.tool)),
        )
    }
}

/// A bound being hit — also not an error.
#[derive(Debug)]
struct BudgetExhausted {
    limit: u32,
}

impl IntoTurnStop for BudgetExhausted {
    fn into_turn_stop(self) -> TurnStop {
        TurnStop::Limited(
            StopDetail::new("acp:max_tokens").with_data("limit", serde_json::json!(self.limit)),
        )
    }
}

/// The ordinary error type: `From` and nothing else — PLX-110's exact posture.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct QuotaExceeded {
    limit: u32,
    observed: u32,
}

impl From<QuotaExceeded> for TurnError {
    fn from(e: QuotaExceeded) -> Self {
        TurnError::structured("app.quota_exceeded", "quota exceeded", &e).retryable(true)
    }
}

#[derive(Clone)]
struct Gate;

#[plexus_macros::activation(
    namespace = "plx112",
    version = "1.0.0",
    description = "The refusal fixture.",
    crate_path = "plexus_core"
)]
impl Gate {
    /// The canonical refusal: the client was asked, the user said no, nothing
    /// failed. This is the shape PLX-105's loopback denial becomes.
    #[plexus_macros::method]
    async fn ask(&self, tool: String) -> Result<Answer, PermissionDenied> {
        Err(PermissionDenied { tool })
    }

    /// A refusal is `Err` on the author's side but not an error on the wire, so
    /// the `Ok` arm of the very same method must be untouched.
    #[plexus_macros::method]
    async fn allow(&self, tool: String) -> Result<Answer, PermissionDenied> {
        Ok(Answer {
            v: tool.len() as u32,
        })
    }

    /// A bound hit.
    #[plexus_macros::method]
    async fn spend(&self, n: u32) -> Result<Answer, BudgetExhausted> {
        Err(BudgetExhausted { limit: n })
    }

    /// The PLX-110 default, re-asserted here rather than assumed: an error type
    /// that says nothing about kind still terminates `Failed`.
    #[plexus_macros::method]
    async fn burst(&self, n: u32) -> Result<Answer, QuotaExceeded> {
        Err(QuotaExceeded {
            limit: 10,
            observed: n,
        })
    }

    /// A unit-returning refusal, so the `UnaryUnit` arm is covered too.
    #[plexus_macros::method]
    async fn poke(&self) -> Result<(), PermissionDenied> {
        Err(PermissionDenied {
            tool: "poke".into(),
        })
    }
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

async fn call(method: &str, params: serde_json::Value) -> Vec<PlexusStreamItem> {
    let stream = Activation::call_arc(Arc::new(Gate), method, params, None, None)
        .await
        .unwrap_or_else(|e| panic!("call_arc({method}) failed: {e:?}"));
    stream.collect().await
}

fn data_items(items: &[PlexusStreamItem]) -> Vec<(String, serde_json::Value)> {
    items
        .iter()
        .filter_map(|i| match i {
            PlexusStreamItem::Data {
                content_type,
                content,
                ..
            } => Some((content_type.clone(), content.clone())),
            _ => None,
        })
        .collect()
}

/// The single terminal of a turn, asserted to be single.
fn terminal(items: &[PlexusStreamItem], expected_type: &str) -> serde_json::Value {
    let data = data_items(items);
    assert_eq!(data.len(), 1, "zero updates, exactly one terminal: {data:?}");
    assert_eq!(data[0].0, expected_type);
    assert!(
        matches!(items.last(), Some(PlexusStreamItem::Done { .. })),
        "the stream still ends with Done"
    );
    data[0].1.clone()
}

// ---------------------------------------------------------------------------
// c1 — a refusal can be said, and it lands as Refused.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_refusal_terminates_the_turn_with_stopkind_refused_via_into_turn_stop() {
    // THE MECHANISM THIS TEST NAMES: `plexus_core::runtime::IntoTurnStop`.
    // `PermissionDenied` implements it and nothing else; the macro emits one
    // fully-qualified call to it and chooses nothing.
    let t = terminal(
        &call("ask", serde_json::json!({ "tool": "bash" })).await,
        "plx112.ask.terminal",
    );

    let stop = &t["stop"];
    assert_eq!(
        stop["kind"],
        serde_json::json!("refused"),
        "RFC 002 §6.6: a considered no is Refused, not Failed"
    );

    // RFC 002 §6.7.1 — a non-Failed terminal MUST NOT carry a structured error.
    assert!(
        stop.get("error").is_none(),
        "a Refused terminal carrying an error is the case a checker MUST \
         reject; got {stop}"
    );

    // §6.5 — the structured reason rides in open detail instead.
    assert_eq!(stop["detail"]["code"], serde_json::json!("acp:permission_denied"));
    assert_eq!(
        stop["detail"]["message"],
        serde_json::json!("the user declined the permission request")
    );
    assert_eq!(stop["detail"]["data"]["tool"], serde_json::json!("bash"));

    // A refusal is not a completion either: no terminal value.
    assert_eq!(t["value"], serde_json::Value::Null);
}

#[tokio::test]
async fn a_unit_returning_method_can_refuse_too() {
    let t = terminal(&call("poke", serde_json::json!({})).await, "plx112.poke.terminal");
    assert_eq!(t["stop"]["kind"], serde_json::json!("refused"));
    assert!(t["stop"].get("error").is_none());
    assert_eq!(t["stop"]["detail"]["code"], serde_json::json!("acp:permission_denied"));
}

#[tokio::test]
async fn the_ok_arm_of_a_refusable_method_is_untouched() {
    let t = terminal(
        &call("allow", serde_json::json!({ "tool": "read" })).await,
        "plx112.allow.terminal",
    );
    assert_eq!(t["stop"]["kind"], serde_json::json!("complete"));
    let v: Answer = serde_json::from_value(t["value"].clone()).unwrap();
    assert_eq!(v, Answer { v: 4 });
}

// ---------------------------------------------------------------------------
// c2 — Failed remains the default.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_error_that_says_nothing_about_kind_still_terminates_failed() {
    // Byte-for-byte the PLX-110 contract: `QuotaExceeded` says only
    // `impl From<QuotaExceeded> for TurnError`, and the blanket `IntoTurnStop`
    // impl carries it to `TurnStop::Failed`. Nothing about this method's
    // observable behaviour changed in PLX-112.
    let t = terminal(
        &call("burst", serde_json::json!({ "n": 37 })).await,
        "plx112.burst.terminal",
    );

    let stop = &t["stop"];
    assert_eq!(stop["kind"], serde_json::json!("failed"));

    let envelope: TurnError = serde_json::from_value(stop["error"].clone())
        .expect("RFC 002 §6.7: Failed MUST carry a structured error");
    assert_eq!(envelope.code, "app.quota_exceeded");
    assert_eq!(envelope.message, "quota exceeded");
    assert_eq!(envelope.retryable, Some(true));
    assert_eq!(
        envelope.details_as::<QuotaExceeded>().unwrap(),
        QuotaExceeded {
            limit: 10,
            observed: 37
        }
    );
    // §6.7.2 — the detail echoes the error's code.
    assert_eq!(stop["detail"]["code"], serde_json::json!("app.quota_exceeded"));
}

#[test]
fn turn_error_itself_still_needs_no_impl_and_still_means_failed() {
    let stop = IntoTurnStop::into_turn_stop(TurnError::new("app.x", "x"));
    assert!(matches!(stop, TurnStop::Failed(_)));
    assert_eq!(
        stop.into_handler_result().unwrap_err().into_stop_reason().kind(),
        StopKind::Failed
    );
}

// ---------------------------------------------------------------------------
// c3 — the neighbours are decided: Limited in, Cancelled deferred.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_bound_lands_as_limited_not_failed() {
    let t = terminal(
        &call("spend", serde_json::json!({ "n": 4096 })).await,
        "plx112.spend.terminal",
    );
    let stop = &t["stop"];
    assert_eq!(stop["kind"], serde_json::json!("limited"));
    assert!(stop.get("error").is_none(), "RFC 002 §6.7.1");
    assert_eq!(stop["detail"]["code"], serde_json::json!("acp:max_tokens"));
    assert_eq!(stop["detail"]["data"]["limit"], serde_json::json!(4096));
}

/// `Cancelled` is NOT reachable from an `Err`, and that is the decision, not an
/// oversight. RFC 002 §6.8 makes cancellation cooperative: the signal must be
/// delivered to the turn. `TurnStop` has three variants and no fourth.
#[test]
fn cancelled_is_deliberately_not_an_err_spelling() {
    let kinds: Vec<StopKind> = [
        TurnStop::Failed(TurnError::new("a", "a")),
        TurnStop::refused("b"),
        TurnStop::limited("c"),
    ]
    .into_iter()
    .map(|s| match s.into_handler_result() {
        Ok(o) => o.into_terminal().0.kind(),
        Err(e) => e.into_stop_reason().kind(),
    })
    .collect();

    assert_eq!(
        kinds,
        vec![StopKind::Failed, StopKind::Refused, StopKind::Limited],
        "the three kinds an Err may mean; Cancelled is the runtime's (§6.8) \
         and Complete is not a stop an Err may claim"
    );
    assert!(
        !kinds.contains(&StopKind::Cancelled),
        "an Err must not be able to claim a cancel signal that was never delivered"
    );
}
