//! PLX-110 (M2·B0) — `#[activation]` can compile a unary method.
//!
//! # What was broken
//!
//! `codegen/handlers.rs::handler_entry` emitted ONE body for every method and
//! unconditionally drove the return value as a `Stream`. A method returning
//! `Result<T, E>` therefore failed to compile:
//!
//! ```text
//! error[E0277]: the trait bound `Result<Answer, String>: futures::Stream` is not satisfied
//! error[E0599]: the method `next` exists for `Pin<Box<Result<Answer, String>>>`,
//!               but its trait bounds were not satisfied
//! ```
//!
//! The trap was an asymmetry: `ir_parse/signature.rs::classify_return` has
//! accepted `Result` since PLX-79 and maps it to a terminal, so reading the IR
//! parser alone told you unary methods already worked. Only the dispatch
//! codegen refused. PLX-110 makes both sides read the same predicates
//! (`ir_parse::signature::dispatch_shape`), so they cannot disagree again.
//!
//! # THE ERROR DECISION, recorded (acceptance criterion c2)
//!
//! **`E: Into<plexus_core::runtime::TurnError>`, and `Err` terminates the turn
//! with `StopKind::Failed`.**
//!
//! Rejected: `E: Display`, because RFC 002 §6.7 says "**Failed** MUST carry a
//! structured error. An implementation MUST NOT flatten a structured error to a
//! string at this boundary". Rejected: `E: Serialize`, because the macro would
//! have to invent the code and message §3.6 item 22 makes mandatory, with no
//! way for the author to override them, and because it silently admits
//! `E = String` and reintroduces the flattened error under a bound that looks
//! structured. `Into<TurnError>` is the narrowest bound under which the macro
//! guesses nothing: the author's `From` impl chooses code, message,
//! retryability and the typed `details` payload.
//!
//! RFC 002 §6.6 (`Refused` is not `Failed`) is NOT satisfied by a mapping here,
//! and is not violated by one either: this arm makes `Err` *mean* `Failed` and
//! leaves a considered "no" with no spelling at all — the same position the
//! streaming arm has always been in. See `a_refusal_has_no_spelling_yet` below.

use async_stream::stream;
use futures::stream::Stream;
use futures::StreamExt;
use plexus_core::plexus::{Activation, PlexusStreamItem};
use plexus_core::runtime::TurnError;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Fixture
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
struct Answer {
    v: u32,
}

/// A domain error with structure worth keeping — the thing a `String` error
/// would have destroyed at the boundary.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct QuotaExceeded {
    limit: u32,
    observed: u32,
}

/// The one impl the `E: Into<TurnError>` decision costs an author. It is also
/// where the author — not the macro — chooses the error code, the message, and
/// what survives to the wire.
impl From<QuotaExceeded> for TurnError {
    fn from(e: QuotaExceeded) -> Self {
        TurnError::structured("app.quota_exceeded", "quota exceeded", &e).retryable(true)
    }
}

#[derive(Clone)]
struct Unary;

#[plexus_macros::activation(
    namespace = "plx110",
    version = "1.0.0",
    description = "The unary-handler fixture.",
    crate_path = "plexus_core"
)]
impl Unary {
    /// A unary method: zero updates, one terminal carrying `Answer`.
    #[plexus_macros::method]
    async fn get(&self, id: String) -> Result<Answer, QuotaExceeded> {
        Ok(Answer {
            v: id.len() as u32,
        })
    }

    /// A unary method that always fails.
    #[plexus_macros::method]
    async fn refuse(&self, n: u32) -> Result<Answer, QuotaExceeded> {
        Err(QuotaExceeded {
            limit: 10,
            observed: n,
        })
    }

    /// A unary method with no meaningful final value: `Result<(), E>`.
    #[plexus_macros::method]
    async fn touch(&self) -> Result<(), QuotaExceeded> {
        Ok(())
    }

    /// The streaming arm, unchanged.
    #[plexus_macros::method]
    async fn tick(&self, times: u32) -> impl Stream<Item = String> + Send + 'static {
        stream! {
            for i in 0..times {
                yield i.to_string();
            }
        }
    }

    /// `Result<impl Stream<Item = T>, E>` — the row `signature.rs`'s doc table
    /// has always claimed to classify, and which did NOT dispatch before this
    /// build (PLX-110's probe reproduced the same `: Stream is not satisfied`
    /// error a plain unary method got).
    #[plexus_macros::method]
    async fn tail(
        &self,
        times: u32,
    ) -> Result<impl Stream<Item = String> + Send + 'static, QuotaExceeded> {
        if times > 10 {
            return Err(QuotaExceeded {
                limit: 10,
                observed: times,
            });
        }
        Ok(stream! {
            for i in 0..times {
                yield format!("t{i}");
            }
        })
    }
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

async fn call(method: &str, params: serde_json::Value) -> Vec<PlexusStreamItem> {
    let stream = Activation::call_arc(Arc::new(Unary), method, params, None, None)
        .await
        .unwrap_or_else(|e| panic!("call_arc({method}) failed: {e:?}"));
    stream.collect().await
}

/// `(content_type, content)` for every `Data` item, in order.
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

// ---------------------------------------------------------------------------
// c1 — a unary method compiles and dispatches.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_unary_method_produces_zero_updates_and_one_terminal_carrying_t() {
    let items = call("get", serde_json::json!({ "id": "abcd" })).await;
    let data = data_items(&items);

    let types: Vec<&str> = data.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(
        types,
        vec!["plx110.get.terminal"],
        "a unary turn emits NO updates and exactly one terminal; got {types:?}"
    );

    assert_eq!(data[0].1["stop"]["kind"], serde_json::json!("complete"));
    // The terminal carries the serialized `T`, not a stream item.
    let v: Answer = serde_json::from_value(data[0].1["value"].clone())
        .expect("the terminal value deserializes back into the declared T");
    assert_eq!(v, Answer { v: 4 });

    assert!(
        matches!(items.last(), Some(PlexusStreamItem::Done { .. })),
        "the stream still ends with Done"
    );
}

#[tokio::test]
async fn a_unary_unit_method_terminates_with_no_value() {
    // RFC 002 §6.4: an implementation MUST NOT substitute a unit type for
    // "declares nothing". `Result<(), E>` must not terminate carrying `null`
    // as a *value* the way `TurnOutcome::serialize(&())` would have.
    let data = data_items(&call("touch", serde_json::json!({})).await);
    assert_eq!(data.len(), 1, "no updates: {data:?}");
    assert_eq!(data[0].0, "plx110.touch.terminal");
    assert_eq!(data[0].1["stop"]["kind"], serde_json::json!("complete"));
    assert_eq!(
        data[0].1["value"],
        serde_json::Value::Null,
        "a unit terminal declares no value"
    );
}

// ---------------------------------------------------------------------------
// c2 — the error path is decided, not incidental.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_err_path_terminates_failed_per_the_recorded_e_into_turnerror_decision() {
    // THE DECISION THIS TEST NAMES: `E: Into<plexus_core::runtime::TurnError>`,
    // and `Err(e)` terminates the turn with `StopKind::Failed`. Not `Refused`,
    // not `Limited` — `TurnError::into_stop_reason` produces `Failed` and this
    // arm hands it a `TurnError` and nothing else.
    let data = data_items(&call("refuse", serde_json::json!({ "n": 37 })).await);
    assert_eq!(data.len(), 1, "no updates on the error path: {data:?}");
    assert_eq!(data[0].0, "plx110.refuse.terminal");

    let stop = &data[0].1["stop"];
    assert_eq!(stop["kind"], serde_json::json!("failed"));

    // RFC 002 §6.7: Failed MUST carry a *structured* error, not a string. The
    // author's own `From` impl chose all of this; the macro chose none of it.
    let envelope: TurnError = serde_json::from_value(stop["error"].clone())
        .expect("the Failed terminal carries the TurnError envelope");
    assert_eq!(envelope.code, "app.quota_exceeded");
    assert_eq!(envelope.message, "quota exceeded");
    assert_eq!(envelope.retryable, Some(true));

    // And the typed domain error survives the round trip intact — the property
    // `E: Display` would have destroyed.
    let domain: QuotaExceeded = envelope
        .details_as()
        .expect("the handler's own error value is in `details`");
    assert_eq!(
        domain,
        QuotaExceeded {
            limit: 10,
            observed: 37
        }
    );
}

#[test]
fn a_refusal_has_no_spelling_yet() {
    // RFC 002 §6.6 says a considered "no" is `Refused` and MUST NOT be
    // conflated with `Failed`. PLX-110 does not conflate them — it never maps a
    // refusal onto `Failed`, it makes `Err` *mean* `Failed`. What it leaves
    // open is that a `#[method]` has no way to say `Refused` at all, because
    // `TurnError::into_stop_reason` hardcodes `StopReason::failed` and
    // `plexus-core` was read-only for this ticket.
    //
    // This is a PRE-EXISTING gap, not one PLX-110 introduced, and this
    // assertion is what will fail loudly if someone closes it in core without
    // teaching the macro the new spelling.
    let stop = TurnError::new("app.denied", "the user said no").into_stop_reason();
    assert_eq!(
        stop.kind(),
        plexus_core::ir::StopKind::Failed,
        "if TurnError can now produce a non-Failed kind, PLX-110's unary Err \
         arm needs revisiting — see the finding in the ticket report"
    );
}

// ---------------------------------------------------------------------------
// c3 — no regression to the streaming arm.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_streaming_arm_is_unchanged() {
    let data = data_items(&call("tick", serde_json::json!({ "times": 3 })).await);
    let types: Vec<&str> = data.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(
        types,
        vec![
            "plx110.tick.update",
            "plx110.tick.update",
            "plx110.tick.update",
            "plx110.tick.terminal",
        ],
        "updates then exactly one terminal, exactly as before PLX-110"
    );
    assert_eq!(data[0].1, serde_json::json!("0"));
    assert_eq!(data[2].1, serde_json::json!("2"));
    assert_eq!(data[3].1["stop"]["kind"], serde_json::json!("complete"));
    assert_eq!(data[3].1["value"], serde_json::Value::Null);
}

#[tokio::test]
async fn a_result_wrapped_stream_still_streams() {
    let data = data_items(&call("tail", serde_json::json!({ "times": 2 })).await);
    let types: Vec<&str> = data.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(
        types,
        vec![
            "plx110.tail.update",
            "plx110.tail.update",
            "plx110.tail.terminal",
        ]
    );
    assert_eq!(data[0].1, serde_json::json!("t0"));
}

#[tokio::test]
async fn a_result_wrapped_stream_that_fails_before_streaming_terminates_failed() {
    let data = data_items(&call("tail", serde_json::json!({ "times": 99 })).await);
    assert_eq!(data.len(), 1, "the stream never opened: {data:?}");
    assert_eq!(data[0].1["stop"]["kind"], serde_json::json!("failed"));
    assert_eq!(
        data[0].1["stop"]["error"]["code"],
        serde_json::json!("app.quota_exceeded")
    );
}

// ---------------------------------------------------------------------------
// c4 — the IR can now tell the two apart.
// ---------------------------------------------------------------------------

#[test]
fn the_ir_distinguishes_a_unary_method_from_a_streaming_one() {
    let ir = Unary::activation_ir();

    let get = ir.method("get").expect("get is in the IR");
    assert!(
        !get.is_streaming(),
        "a `Result<T, E>` method declares NO update type"
    );
    assert!(
        get.updates.is_none(),
        "a unary method is a terminal-without-updates"
    );
    let terminal = get
        .terminal
        .as_ref()
        .expect("a unary method declares its terminal type");
    assert_eq!(terminal.type_name(), "Answer");

    let touch = ir.method("touch").expect("touch is in the IR");
    assert!(!touch.is_streaming());
    assert!(
        touch.terminal.is_none(),
        "RFC 002 §6.4: the unit type is absence, never a declared `()` terminal"
    );

    let tick = ir.method("tick").expect("tick is in the IR");
    assert!(tick.is_streaming(), "a stream method declares an update type");
    assert!(
        tick.terminal.is_none(),
        "a streaming method has no distinct terminal type"
    );

    // `Result<impl Stream<Item = T>, E>` classifies as streaming, not unary —
    // the row `signature.rs`'s doc table has claimed all along.
    let tail = ir.method("tail").expect("tail is in the IR");
    assert!(tail.is_streaming());
    assert!(tail.terminal.is_none());

    // THE FACT PLX-107 ASKED FOR, asserted rather than asserted-about:
    // `is_streaming` is a faithful reading of the return type in BOTH
    // directions now that both shapes are writable. It became *available* as a
    // routing signal here; it becomes *authoritative* only once the ~136
    // one-shot methods still spelled `impl Stream` are converted (PLX-109's
    // conversion queue).
    let streaming: Vec<(&str, bool)> = ir
        .methods
        .iter()
        .map(|m| (m.name.as_str(), m.is_streaming()))
        .collect();
    assert_eq!(
        streaming,
        vec![
            ("get", false),
            ("refuse", false),
            ("touch", false),
            ("tick", true),
            ("tail", true),
        ]
    );
}

// ---------------------------------------------------------------------------
// Requirement 4 — additive to `MethodSchema`; routing is untouched.
// ---------------------------------------------------------------------------

#[test]
fn method_schema_streaming_stays_the_declaration_for_unary_methods_too() {
    // PLX-107's invariant, extended over the new shape: `MethodSchema.streaming`
    // is the author's opt-in transport decision, and plexus-transport reads it
    // (and nothing derived) to choose SSE over JSON. No method here declares
    // `streaming`, so every flag is false — including the two that genuinely
    // stream. If this ever flips for `tick`/`tail`, routing has been moved onto
    // the derived flag and PLX-107's transport tests will fail too.
    let schema = Unary.plugin_schema();
    for m in &schema.methods {
        assert!(
            !m.streaming,
            "`{}` declared no `streaming` attribute, so the schema flag must be false",
            m.name
        );
    }

    // PLX-107's other invariant: `s.streaming` implies `m.is_streaming()`. It
    // holds vacuously here, and must not become false-able by the unary arm.
    let ir = Unary::activation_ir();
    for m in &schema.methods {
        if m.streaming {
            assert!(ir.method(&m.name).unwrap().is_streaming());
        }
    }

    // Additive, not substitutive: a unary method now advertises a return schema
    // where before PLX-110 it would have had none (it had no stream item type).
    let get = schema
        .methods
        .iter()
        .find(|m| m.name == "get")
        .expect("get is in the schema");
    assert!(
        get.returns.is_some(),
        "a unary method advertises its terminal type as the schema's return type"
    );
}
