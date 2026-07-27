//! PLX-146 — a generated activation can declare a callback AND answer one.
//!
//! # What was broken, and why reading the code said otherwise
//!
//! Two blockers, both found by compile probe rather than by reading, and both
//! invisible from the IR parser:
//!
//! 1. **`#[activation]` could not take a `Client<C>` at all.** A method with a
//!    `client: Client<(Permission,)>` parameter failed with
//!
//!    ```text
//!    error[E0277]: the trait bound `Client<(Permission,)>: Serialize` is not satisfied
//!    error[E0277]: the trait bound `Client<(Permission,)>: Deserialize<'_>` is not satisfied
//!    ```
//!
//!    `ir_parse/signature.rs` strips the handle correctly — it has since
//!    PLX-79, and `plx79_ir_parse_tests.rs` proves it — but that is the *IR*
//!    parser. The **legacy** parser in `parse.rs`, which is what drives
//!    `codegen/handlers.rs` and `codegen/method_enum.rs`, had no `Client`
//!    branch, so the handle fell through to the wire-parameter tail and was
//!    schema'd and deserialized like a `String`. This is the third instance in
//!    this project of the same class of bug (PLX-110 was the second): the IR
//!    parser accepts a shape the dispatch codegen refuses, so the feature looks
//!    implemented from one side and is unreachable from the other.
//!
//! 2. **The generated activation dropped the `TurnControl`.**
//!    `codegen/activation.rs`'s `call_arc` passed `live: None` to
//!    `turn_stream_to_plexus_stream`. The callback was still projected onto the
//!    wire as a `PlexusStreamItem::Request`, so from the outside the mechanism
//!    looked live — but no `TurnControl` was retained under the turn's id, so
//!    no answer could ever be routed back and the handler's
//!    `client.request_permission_async(..).await` never resolved.
//!
//! Together: **a generated activation could declare a callback and could never
//! answer one.** Every server-to-client callback in the fleet was reachable
//! only from a hand-written `DeclaredHandler`.
//!
//! # Why every assertion here carries a timeout
//!
//! **A test that hangs is a failing test.** The failure mode of blocker 2 is a
//! callback that never resolves — a test that waits forever reports nothing at
//! all, it just wedges the suite. So every await below is inside
//! `tokio::time::timeout`, and the timeout expiring is an explicit assertion
//! failure that names the blocker.
//!
//! # What keeps the PLX-102 guarantee intact
//!
//! `Turn<C>::client()` is still the only minter, and `TurnContext` still cannot
//! mint a client at all. The macro reads `C` out of the `client: Client<C>`
//! parameter, and **the same tokens** are what `ir_parse` emits as
//! `MethodIr::callbacks`. One signature, one `C`, two artifacts that cannot
//! disagree — which is what `a_declared_handle_is_the_declaration` asserts
//! directly.

use futures::StreamExt;
use plexus_core::capability::{Client, PermissionOutcome, PermissionRequest};
use plexus_core::plexus::{Activation, PlexusStreamItem};
use plexus_core::runtime::{CallbackId, TurnError};
use std::sync::Arc;
use std::time::Duration;

/// Long enough that a slow machine does not flake, short enough that the
/// pre-fix hang is reported as a failure in seconds rather than never.
const BOUND: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------------------
// Fixture
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct Guarded;

#[plexus_macros::activation(
    namespace = "plx146",
    version = "1.0.0",
    description = "The generated-callback fixture.",
    crate_path = "plexus_core"
)]
impl Guarded {
    /// The shape the whole ticket is about: a generated method that takes a
    /// capability handle and issues a callback through it.
    ///
    /// Before PLX-146 this did not compile.
    #[plexus_macros::method]
    async fn ask(
        &self,
        path: String,
        client: Client<(plexus_core::capability::Permission,)>,
    ) -> Result<PermissionOutcome, TurnError> {
        client
            .request_permission_async(PermissionRequest {
                operation: "fs/write_text_file".into(),
                rationale: Some(format!("write {path}")),
            })
            .await
            .map_err(|e| TurnError::new("app.callback_failed", e.to_string()))
    }

    /// The handle in front of the wire parameter rather than behind it.
    ///
    /// `call_args` is positional and was never signature-order-aware, so this
    /// is not a redundant fixture: appending the handle instead of splicing it
    /// at its recorded index compiles whenever the two types unify and then
    /// passes the arguments **swapped**.
    #[plexus_macros::method]
    async fn ask_handle_first(
        &self,
        client: Client<(plexus_core::capability::Permission,)>,
        path: String,
    ) -> Result<String, TurnError> {
        let outcome = client
            .request_permission_async(PermissionRequest {
                operation: "fs/write_text_file".into(),
                rationale: None,
            })
            .await
            .map_err(|e| TurnError::new("app.callback_failed", e.to_string()))?;
        Ok(format!("{path}:{outcome:?}"))
    }

    /// A method with no handle at all, to pin that nothing changed for the
    /// overwhelming majority of methods that declare no callbacks.
    #[plexus_macros::method]
    async fn plain(&self, path: String) -> Result<String, TurnError> {
        Ok(path)
    }
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// Drive a turn to completion, answering the first callback it raises.
///
/// Deliberately incremental rather than `stream.collect().await`: collecting
/// would deadlock by construction, because the terminal cannot arrive until the
/// callback is answered and the callback cannot be answered until it has been
/// observed. That deadlock IS blocker 2's failure mode, so the harness is
/// written to expose it rather than to sit in it.
async fn ask_and_answer(
    method: &str,
    params: serde_json::Value,
    answer: serde_json::Value,
) -> serde_json::Value {
    let mut stream = Activation::call_arc(Arc::new(Guarded), method, params, None, None)
        .await
        .unwrap_or_else(|e| panic!("call_arc({method}) failed: {e:?}"));

    // 1. The callback must reach the wire.
    let first = tokio::time::timeout(BOUND, stream.next())
        .await
        .expect("PLX-146 blocker 2: no callback was emitted within the bound")
        .expect("the stream ended before emitting anything");

    let request_id = match first {
        PlexusStreamItem::Request { request_id, .. } => request_id,
        other => panic!("expected a callback Request, got {other:?}"),
    };

    // 2. A responder answers it. The legacy wire carries "<turn>:<seq>".
    //
    // NOTE the assertion is on *this turn's identity*, not on a count. The
    // generated registry is a per-monomorphization `static`, so every
    // concurrently-running test in this binary shares it and `len()` is not a
    // per-turn fact. `respond` succeeding is the precise claim anyway: it
    // proves a `TurnControl` was retained under exactly this turn id, which is
    // the fact `live: None` destroyed.
    let id = parse_callback_id(&request_id);
    Guarded::__plexus_live_turns().respond(id, answer).expect(
        "PLX-146 blocker 2: the generated activation kept no TurnControl for \
         this turn, so nothing can route an answer back to it",
    );

    // 3. The turn must then complete. Before PLX-146 this waited forever.
    let rest: Vec<PlexusStreamItem> = tokio::time::timeout(BOUND, stream.collect())
        .await
        .expect(
            "PLX-146 blocker 2: the turn never completed after the callback was \
             answered — the handler is still parked on request_permission_async",
        );

    let terminal = rest
        .iter()
        .find_map(|i| match i {
            PlexusStreamItem::Data {
                content_type,
                content,
                ..
            } if content_type.ends_with(".terminal") => Some(content.clone()),
            _ => None,
        })
        .expect("exactly one terminal Data item");

    // Eviction, asserted per-turn for the same reason: answering the SAME id
    // again must now report the turn has ended. A registry that never evicted
    // would leak a `TurnControl` per turn forever.
    let err = Guarded::__plexus_live_turns()
        .respond(id, serde_json::json!(null))
        .expect_err("a resolved turn must be gone from the registry");
    assert!(
        matches!(err, plexus_core::runtime::RespondError::TurnEnded(_)),
        "expected TurnEnded for a completed turn, got {err:?}"
    );

    terminal
}

/// The legacy wire carries a callback id as `"<turn>:<seq>"`.
fn parse_callback_id(s: &str) -> CallbackId {
    let (turn, seq) = s.split_once(':').expect("a <turn>:<seq> id");
    CallbackId::new(
        serde_json::from_value(serde_json::json!(turn)).unwrap(),
        seq.parse().unwrap(),
    )
}

// ---------------------------------------------------------------------------
// c1 — the handle is a handle, not a parameter.
// ---------------------------------------------------------------------------

/// The fixture above compiling at all is half of c1. This is the other half:
/// the handle must be absent from the method's declared wire params.
///
/// If it were merely *tolerated* rather than stripped, it would still show up
/// here — and it would be in the method-hash preimage, moving the hash of every
/// method that takes one.
#[test]
fn the_capability_handle_is_not_a_declared_wire_param() {
    let ir = Guarded::activation_ir();

    let ask = ir.method("ask").expect("the ask method");
    let names: Vec<&str> = ask.params.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["path"],
        "the capability handle must not be a wire parameter"
    );

    let handle_first = ir
        .method("ask_handle_first")
        .expect("the ask_handle_first method");
    let names: Vec<&str> = handle_first.params.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["path"],
        "handle position must not change which params are wire params"
    );
}

/// The declaration and the minted handle come from one `C`.
///
/// This is what makes the `Turn::__declared_by_macro` seam safe: the tokens the
/// codegen mints the client from are the tokens `ir_parse` turned into
/// `callbacks`, so a generated handler cannot call a capability its own IR does
/// not advertise.
#[test]
fn a_declared_handle_is_the_declaration() {
    let ir = Guarded::activation_ir();

    let ask = ir.method("ask").expect("the ask method");
    assert_eq!(
        ask.callbacks,
        Client::<(plexus_core::capability::Permission,)>::callbacks(),
        "MethodIr::callbacks must be derived from the same C the handle is"
    );
    assert_eq!(ask.callbacks.len(), 1);
    assert_eq!(ask.callbacks[0].name, "session/request_permission");

    let plain = ir.method("plain").expect("the plain method");
    assert!(
        plain.callbacks.is_empty(),
        "a method with no handle declares no callbacks"
    );
}

// ---------------------------------------------------------------------------
// c2 — a callback can be answered.
// ---------------------------------------------------------------------------

/// The ticket's central claim, end to end, under a bound.
#[tokio::test]
async fn a_generated_activation_issues_a_callback_and_the_answer_completes_the_turn() {
    let terminal = ask_and_answer(
        "ask",
        serde_json::json!({ "path": "/tmp/x" }),
        serde_json::json!({ "outcome": "allow" }),
    )
    .await;

    assert_eq!(
        terminal["stop"]["kind"], "complete",
        "the turn must complete, not fail"
    );
    assert_eq!(
        terminal["value"],
        serde_json::json!({ "outcome": "allow" }),
        "the terminal must carry the answer the responder gave"
    );
}

/// A denial is still an *answer*: the callback resolves and the turn completes.
///
/// Note what this does NOT assert. `StopKind::Refused` is PLX-112's spelling,
/// and reaching it from here would require the handler to map the outcome onto
/// `TurnOutcome::Refused`; this fixture returns the outcome as a value, so a
/// denial completes. That is the honest current behaviour and it is asserted as
/// such rather than papered over.
#[tokio::test]
async fn a_denial_is_delivered_as_an_answer_and_the_turn_still_completes() {
    let terminal = ask_and_answer(
        "ask",
        serde_json::json!({ "path": "/tmp/x" }),
        serde_json::json!({ "outcome": "deny" }),
    )
    .await;

    assert_eq!(terminal["stop"]["kind"], "complete");
    assert_eq!(terminal["value"], serde_json::json!({ "outcome": "deny" }));
}

/// The handle must arrive in the argument position the author wrote it in.
///
/// With the handle appended rather than spliced, this method receives `path`
/// where it expects `client`.
#[tokio::test]
async fn the_handle_is_passed_at_the_position_the_signature_declares() {
    let terminal = ask_and_answer(
        "ask_handle_first",
        serde_json::json!({ "path": "/tmp/y" }),
        serde_json::json!({ "outcome": "allow" }),
    )
    .await;

    assert_eq!(terminal["stop"]["kind"], "complete");
    assert_eq!(
        terminal["value"], "/tmp/y:Allow",
        "the wire param and the handle must not be swapped"
    );
}

/// Two turns in flight at once must not cross-talk.
///
/// The generated registry is a per-monomorphization `static`, not a per-instance
/// field, so this is the assertion that keys-by-`TurnId` is actually sufficient.
#[tokio::test]
async fn two_concurrent_generated_turns_do_not_cross_talk() {
    let mut a = Activation::call_arc(
        Arc::new(Guarded),
        "ask",
        serde_json::json!({ "path": "/a" }),
        None,
        None,
    )
    .await
    .unwrap();
    let mut b = Activation::call_arc(
        Arc::new(Guarded),
        "ask",
        serde_json::json!({ "path": "/b" }),
        None,
        None,
    )
    .await
    .unwrap();

    let id_of = |item: Option<PlexusStreamItem>| match item.unwrap() {
        PlexusStreamItem::Request { request_id, .. } => request_id,
        other => panic!("expected a Request, got {other:?}"),
    };

    let ida = id_of(
        tokio::time::timeout(BOUND, a.next())
            .await
            .expect("turn a emitted no callback"),
    );
    let idb = id_of(
        tokio::time::timeout(BOUND, b.next())
            .await
            .expect("turn b emitted no callback"),
    );

    assert_ne!(ida, idb, "two turns must not share a callback id");

    // Answer b FIRST, out of order, with a value only b could return — so a
    // registry that ignored turn identity would visibly swap the two answers.
    Guarded::__plexus_live_turns()
        .respond(parse_callback_id(&idb), serde_json::json!({ "outcome": "deny" }))
        .unwrap();
    Guarded::__plexus_live_turns()
        .respond(parse_callback_id(&ida), serde_json::json!({ "outcome": "allow" }))
        .unwrap();

    let rest_a: Vec<PlexusStreamItem> = tokio::time::timeout(BOUND, a.collect())
        .await
        .expect("turn a never completed");
    let rest_b: Vec<PlexusStreamItem> = tokio::time::timeout(BOUND, b.collect())
        .await
        .expect("turn b never completed");

    let value = |items: &[PlexusStreamItem]| {
        items
            .iter()
            .find_map(|i| match i {
                PlexusStreamItem::Data {
                    content_type,
                    content,
                    ..
                } if content_type.ends_with(".terminal") => Some(content["value"].clone()),
                _ => None,
            })
            .expect("a terminal")
    };

    assert_eq!(value(&rest_a), serde_json::json!({ "outcome": "allow" }));
    assert_eq!(value(&rest_b), serde_json::json!({ "outcome": "deny" }));
}

/// A method that declares no handle is untouched — same dispatch, no registry
/// interaction beyond the eviction every turn now gets.
#[tokio::test]
async fn a_method_without_a_handle_is_unchanged() {
    let stream = Activation::call_arc(
        Arc::new(Guarded),
        "plain",
        serde_json::json!({ "path": "/z" }),
        None,
        None,
    )
    .await
    .unwrap();
    let items: Vec<PlexusStreamItem> = tokio::time::timeout(BOUND, stream.collect())
        .await
        .expect("a method with no callback must not block on anything");

    assert!(
        !items
            .iter()
            .any(|i| matches!(i, PlexusStreamItem::Request { .. })),
        "a method that declares no callbacks must emit no Request"
    );
}
