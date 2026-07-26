//! PLX-91 (M1·F) — the switchover's own acceptance tests.
//!
//! Three things are asserted here, one per acceptance criterion the switchover
//! owns:
//!
//! 1. **The macro emits data, not a dispatcher.** `#[activation]` now produces
//!    an `ActivationIr` and a `HandlerTable<Self>`; dispatch is
//!    `plexus_core::runtime::entry_with_state`, and there is no generated
//!    `match method { "name" => … }` arm left for a non-bidirectional method.
//! 2. **The turn runtime is what actually runs.** A method dispatched through
//!    `Activation::call_arc` produces the turn projection — updates then a
//!    terminal — rather than a `wrap_stream` template's bare data items.
//! 3. **The IR and the `MethodSchema` agree** (requirement 4 as amended
//!    2026-07-26). The two are computed alongside each other rather than one
//!    from the other, so the invariant that they cannot drift is a *test*
//!    rather than a construction. Where they overlap they must agree; where
//!    they do not overlap the ticket records why, and this test does not
//!    pretend otherwise.
//!
//! PLX-107 settled the one item that was left as a "known gap" here,
//! `streaming`, and it is now an asserted invariant rather than a pinned
//! disagreement. The finding was that the two fields are not two answers to one
//! question: `MethodIr::is_streaming()` asks "does the turn declare an update
//! item type?" and `MethodSchema.streaming` asks "should the transport serve
//! SSE instead of JSON?". Equating them would have flipped ~154 live methods'
//! wire format. See `the_ir_and_the_method_schema_agree_where_they_overlap` and
//! `method_schema_streaming_is_the_declaration_not_the_return_type` below.

use async_stream::stream;
use futures::stream::Stream;
use futures::StreamExt;
use plexus_core::plexus::{Activation, AuthContext, PlexusStreamItem};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Fixture: one public method with a required and an optional parameter, one
// auth-gated method, and one method with no parameters at all.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct Switched;

#[plexus_macros::activation(
    namespace = "plx91",
    version = "1.0.0",
    description = "The switchover fixture.",
    auth = "mixed",
    crate_path = "plexus_core"
)]
impl Switched {
    /// Echo a message a number of times.
    #[plexus_macros::method(public)]
    async fn echo(
        &self,
        msg: String,
        times: Option<u32>,
    ) -> impl Stream<Item = String> + Send + 'static {
        let n = times.unwrap_or(1);
        stream! {
            for _ in 0..n {
                yield msg.clone();
            }
        }
    }

    /// Report who is calling.
    #[plexus_macros::method]
    async fn whoami(&self, auth: &AuthContext) -> impl Stream<Item = String> + Send + 'static {
        let id = auth.user_id.clone();
        stream! { yield id; }
    }

    /// Take nothing, say something.
    #[plexus_macros::method(public)]
    async fn ping(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "pong".to_string(); }
    }
}

/// A hub over the fixture above, so the exported document exercises a child
/// edge as well as a method list.
#[derive(Clone)]
struct Plx91Hub {
    leaf: Switched,
}

#[plexus_macros::activation(
    namespace = "plx91_hub",
    version = "1.0.0",
    description = "The switchover fixture's parent.",
    crate_path = "plexus_core"
)]
impl Plx91Hub {
    /// Say hello.
    #[plexus_macros::method(public)]
    async fn hello(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "hello".to_string(); }
    }

    /// The switched leaf.
    #[plexus_macros::child]
    fn leaf(&self) -> Switched {
        self.leaf.clone()
    }
}

// ---------------------------------------------------------------------------
// 1. The macro emits data, not a dispatcher.
// ---------------------------------------------------------------------------

#[test]
fn the_macro_emits_an_activation_ir_literal() {
    let ir = Switched::activation_ir();

    assert_eq!(ir.namespace, "plx91");
    assert_eq!(ir.version, "1.0.0");

    let names: Vec<&str> = ir.methods.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(names, vec!["echo", "whoami", "ping"]);

    let echo = ir.method("echo").expect("echo is in the IR");
    assert_eq!(echo.dotted_id, "plx91.echo");
    assert_eq!(echo.description, "Echo a message a number of times.");
    // `Option<u32>` is spelled `required: false` over the inner type, not by
    // wrapping the schema — `ParamIr`'s contract.
    let params: Vec<(&str, bool)> = echo
        .params
        .iter()
        .map(|p| (p.name.as_str(), p.required))
        .collect();
    assert_eq!(params, vec![("msg", true), ("times", false)]);
    // A streaming method declares an update item type and no distinct terminal.
    assert!(echo.is_streaming());

    // The auth posture is the one the legacy dispatch actually enforced, not
    // the IR's bare default-deny: `#[method(public)]` is Public, and the
    // `auth: &AuthContext` signature is Required.
    use plexus_core::ir::AuthRequirementIr;
    assert_eq!(echo.auth, AuthRequirementIr::Public);
    assert_eq!(
        ir.method("whoami").unwrap().auth,
        AuthRequirementIr::Required
    );

    // The document is hashed — `recompute_hashes` ran at construction.
    assert!(ir.ir_hash.is_some(), "the IR carries its document digest");
}

#[test]
fn the_macro_emits_a_handler_table() {
    let table = Switched::__plexus_handler_table();
    assert_eq!(table.order(), &["echo", "whoami", "ping"]);
    assert_eq!(table.len(), 3);
    assert!(table.contains("echo"));
    assert!(!table.contains("not_a_method"));
}

#[test]
fn every_ir_method_has_a_handler_and_vice_versa() {
    let ir = Switched::activation_ir();
    let table = Switched::__plexus_handler_table();
    for m in &ir.methods {
        assert!(
            table.contains(&m.name),
            "IR method `{}` has no handler — `entry` would answer HandlerMissing",
            m.name
        );
    }
    for name in table.order() {
        assert!(
            ir.method(name).is_some(),
            "handler `{name}` has no IR method — it is unreachable through `entry`"
        );
    }
}

// ---------------------------------------------------------------------------
// 2. The turn runtime is what actually runs.
// ---------------------------------------------------------------------------

async fn call(method: &str, params: serde_json::Value) -> Vec<PlexusStreamItem> {
    let stream = Activation::call_arc(Arc::new(Switched), method, params, None, None)
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

#[tokio::test]
async fn a_switched_method_runs_as_a_turn() {
    let items = call("echo", serde_json::json!({ "msg": "hi", "times": 2 })).await;
    let data = data_items(&items);

    // The turn projection: one `.update` per stream item, then exactly one
    // `.terminal`, then `Done`. Both the exactly-once terminal and its
    // last-ness are structural properties of `runtime::entry`'s loop, not of
    // anything this macro emits.
    let types: Vec<&str> = data.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(
        types,
        vec!["plx91.echo.update", "plx91.echo.update", "plx91.echo.terminal"],
        "got {types:?}"
    );
    assert_eq!(data[0].1, serde_json::json!("hi"));
    assert_eq!(data[1].1, serde_json::json!("hi"));
    assert!(
        matches!(items.last(), Some(PlexusStreamItem::Done { .. })),
        "the stream still ends with Done"
    );

    let stop = &data[2].1["stop"];
    assert_eq!(stop["kind"], serde_json::json!("complete"));
}

#[tokio::test]
async fn a_method_with_no_params_runs() {
    let data = data_items(&call("ping", serde_json::json!({})).await);
    assert_eq!(data[0].1, serde_json::json!("pong"));
}

#[tokio::test]
async fn auth_injection_comes_from_the_runtime_not_a_template() {
    // `AuthRequirementIr::Required` is enforced by `entry`'s pre-flight, before
    // any handler exists — so this is an `Err`, not a failed turn.
    let err = Activation::call_arc(
        Arc::new(Switched),
        "whoami",
        serde_json::json!({}),
        None,
        None,
    )
    .await
    .err()
    .expect("a Required method with no auth context must not open a turn");
    assert!(
        matches!(err, plexus_core::plexus::PlexusError::Unauthenticated(_)),
        "got {err:?}"
    );

    let ctx = AuthContext::new(
        "alice".into(),
        "sess".into(),
        vec![],
        serde_json::Value::Null,
    );
    let stream = Activation::call_arc(
        Arc::new(Switched),
        "whoami",
        serde_json::json!({}),
        Some(&ctx),
        None,
    )
    .await
    .expect("an authenticated call opens a turn");
    let items: Vec<PlexusStreamItem> = stream.collect().await;
    assert_eq!(data_items(&items)[0].1, serde_json::json!("alice"));
}

#[tokio::test]
async fn a_decode_failure_is_a_structured_terminal_on_a_real_turn() {
    // Before the switchover a missing required field was a pre-flight
    // `PlexusError::InvalidParams` with no turn behind it. It is now a turn
    // that resolves `Failed` with a structured error — the update/terminal
    // split doing its job.
    let items = call("echo", serde_json::json!({})).await;
    let data = data_items(&items);
    assert_eq!(data.len(), 1, "no updates, just the terminal: {data:?}");
    assert_eq!(data[0].0, "plx91.echo.terminal");
    let stop = &data[0].1["stop"];
    assert_eq!(stop["kind"], serde_json::json!("failed"));
    assert!(
        stop.to_string().contains("missing field: msg"),
        "the terminal names the missing field: {stop}"
    );
}

// ---------------------------------------------------------------------------
// 3. The IR and the MethodSchema agree where they overlap.
//
// Requirement 4, as amended by the orchestrator on 2026-07-26. The original
// requirement — schema DERIVED FROM the IR, so the two "cannot disagree" — is
// not achievable at this build:
//
//   (a) Six `MethodSchema` fields have no IR representation at all
//       (`requires_credential`, `auth_posture`, `params_meta`, `credentials`,
//       the `bidirectional`/`request_type`/`response_type` triple, and `hash`);
//       four of the six belong to surface PLX-92 is about to delete.
//   (b) REQ-6 requires the activation's request-context fields be MERGED into
//       each method's `params` with `x-plexus-source`; RFC 002 §6.9.5 states
//       they MUST NOT be. The two schema surfaces are deliberately different
//       objects.
//
// So the two are computed alongside each other, and this is the test that
// pins them together on every field they both carry.
// ---------------------------------------------------------------------------

#[test]
fn the_ir_and_the_method_schema_agree_where_they_overlap() {
    let ir = Switched::activation_ir();
    let schema = Switched.plugin_schema();

    assert_eq!(schema.namespace, ir.namespace, "namespace");
    assert_eq!(schema.version, ir.version, "version");
    assert_eq!(schema.description, ir.description, "description");

    // Every IR method is a schema method, in the same order, with the same
    // name and description.
    let schema_methods: Vec<&plexus_core::plexus::MethodSchema> = schema
        .methods
        .iter()
        .filter(|m| ir.method(&m.name).is_some())
        .collect();
    assert_eq!(
        schema_methods.len(),
        ir.methods.len(),
        "every IR method must appear in the MethodSchema list (schema has {:?})",
        schema.methods.iter().map(|m| &m.name).collect::<Vec<_>>()
    );

    for m in &ir.methods {
        let s = schema
            .methods
            .iter()
            .find(|s| s.name == m.name)
            .unwrap_or_else(|| panic!("method `{}` is in the IR but not the schema", m.name));

        assert_eq!(s.description, m.description, "{}: description", m.name);

        // `MethodSchema.public` and `MethodIr.auth` are the same decision seen
        // from two sides: a Public method is exactly a non-auth-gated one.
        use plexus_core::ir::AuthRequirementIr;
        let ir_is_public = matches!(m.auth, AuthRequirementIr::Public);
        assert_eq!(
            s.public, ir_is_public,
            "{}: MethodSchema.public ({}) disagrees with MethodIr.auth ({:?})",
            m.name, s.public, m.auth
        );

        // `streaming` — PLX-107 settled this. It is no longer a recorded gap;
        // it is an asserted invariant, and the shape of the invariant is the
        // finding.
        //
        // PLX-91 recorded the two fields as disagreeing, and PLX-107 was
        // dispatched to make the derived value win. Investigating it falsified
        // the premise. The two fields are not two answers to one question, they
        // are answers to two different questions:
        //
        //   - `MethodIr::is_streaming()` == `updates.is_some()` — "does the
        //     turn declare an update item type?" A fact about the return type.
        //   - `MethodSchema.streaming` — "should the transport hold the
        //     connection open and serve SSE instead of JSON?" A decision the
        //     author makes with `#[method(streaming)]`.
        //
        // They cannot be made equal, because in this DX the transport envelope
        // is *always* a stream: a one-shot method and a many-shot method are
        // both spelled `impl Stream<Item = T>`, the first yielding a single
        // `Data` then `Done`. So `is_streaming()` is true for nearly every
        // method ever written here (every method in this fixture included) and
        // is not a routing signal. Equating them would have flipped ~154 live
        // methods across plexus-idp, plexus-registry and plexus-substrate from
        // JSON to SSE — see the record at
        // `plexus-macros/src/codegen/method_enum.rs` (`streaming_flags`) and
        // the boundary test in plexus-transport
        // (`tests/plx107_streaming_routing_tests.rs`).
        //
        // What must hold, and is asserted, is the implication: SSE is only ever
        // chosen for a method that genuinely has an update item type to send.
        // A schema that claims `streaming` over a return type with no update
        // item would be a real defect — the transport would open an SSE stream
        // for a turn that has nothing to stream.
        if s.streaming {
            assert!(
                m.is_streaming(),
                "{}: MethodSchema.streaming is true but the IR sees no update item type",
                m.name
            );
        }

        // `MethodSchema.http_method` and `MethodIr.http_method` are the same
        // verb, spelled by two enums.
        assert_eq!(
            format!("{:?}", s.http_method).to_uppercase(),
            format!("{:?}", m.http_method).to_uppercase(),
            "{}: http_method",
            m.name
        );

        // Parameter names and requiredness. The schema spells a parameter list
        // as a JSON Schema object; the IR spells it as `ParamIr`s. Both must
        // name the same parameters, and agree on which are required.
        let params_value: serde_json::Value = s
            .params
            .as_ref()
            .map(|sch| sch.as_value().clone())
            .unwrap_or(serde_json::Value::Null);

        let required: Vec<String> = params_value
            .get("required")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();

        for p in &m.params {
            assert!(
                params_value
                    .get("properties")
                    .and_then(|v| v.get(&p.name))
                    .is_some(),
                "{}: IR param `{}` is missing from MethodSchema.params ({params_value})",
                m.name,
                p.name
            );
            assert_eq!(
                required.contains(&p.name),
                p.required,
                "{}: param `{}` requiredness disagrees (schema required = {:?})",
                m.name,
                p.name,
                required
            );
        }

        // ...and nothing the schema calls a parameter is missing from the IR.
        if let Some(props) = params_value.get("properties").and_then(|v| v.as_object()) {
            for name in props.keys() {
                assert!(
                    m.params.iter().any(|p| &p.name == name),
                    "{}: MethodSchema parameter `{}` is missing from the IR",
                    m.name,
                    name
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The `call(&self, ..)` residual, pinned so it cannot regress into silence.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_borrowed_call_of_a_turn_native_method_says_why_it_cannot() {
    // PLX-97 left exactly one residual: a DIRECT `Activation::call(&self, ..)`
    // has no owned handle, so it cannot run a handler the turn's `'static`
    // stream outlives. It must say so, loudly, naming the replacement — the
    // one thing worse than this error would be silently dispatching some other
    // way.
    let err = Activation::call(&Switched, "ping", serde_json::json!({}), None, None)
        .await
        .err()
        .expect("a borrowed call of a turn-native method must not succeed");
    let msg = format!("{err:?}");
    assert!(msg.contains("call_arc"), "the error names the fix: {msg}");
    assert!(msg.contains("plx91.ping"), "the error names the method: {msg}");
}

// ---------------------------------------------------------------------------
// The independent-implementation check: export the switched macro's Connectome
// so `connectome-hs` can pass its own conformance checker over it.
//
// The document is written to `$PLX91_CONNECTOME_OUT` (or the temp dir), which
// is what the acceptance evidence quotes. `connectome-hs` was written from
// CONNECTOME RFC 001 rather than from plexus-core, so its verdict over this
// document is evidence about the *macro's output*, not about a shared
// codebase.
// ---------------------------------------------------------------------------

#[test]
fn the_switched_macro_exports_a_connectome() {
    let ir = Plx91Hub::activation_ir();

    // A root document: a namespace, a version, methods, and one static child
    // edge carrying the leaf's whole embedded subtree.
    assert_eq!(ir.namespace, "plx91_hub");
    assert_eq!(ir.children.len(), 1);
    assert!(ir.ir_hash.is_some());

    let json = serde_json::to_string_pretty(ir).expect("the IR serializes");

    // §3.4 round-trip: what the macro emits decodes back to an equal document.
    let back: plexus_core::ir::ActivationIr =
        serde_json::from_str(&json).expect("the document round-trips");
    assert_eq!(&back, ir);

    let out = std::env::var("PLX91_CONNECTOME_OUT")
        .unwrap_or_else(|_| {
            std::env::temp_dir()
                .join("plx91_connectome.json")
                .to_string_lossy()
                .into_owned()
        });
    std::fs::write(&out, &json).unwrap_or_else(|e| panic!("could not write {out}: {e}"));
    eprintln!("PLX-91: Connectome written to {out}");
}

// ---------------------------------------------------------------------------
// PLX-107. The macro end of the routing chain: `MethodSchema.streaming` is the
// author's declaration, verbatim, and the return type does not vote.
//
// Every method in this fixture returns `impl Stream<Item = String>` — `echo`
// genuinely yields many items — and none of them declares `#[method(streaming)]`.
// Under the candidate `m.streaming || m.stream_item_type.is_some()` every one of
// them would come out `streaming: true` and plexus-transport would serve all of
// them as SSE instead of JSON. That is the fleet-wide break PLX-107 refused; see
// `src/codegen/method_enum.rs` (`streaming_flags`) for the full record, and
// `plexus-transport/tests/http_gateway_streaming_tests.rs` for the boundary
// where it would have been observable.
// ---------------------------------------------------------------------------

#[test]
fn method_schema_streaming_is_the_declaration_not_the_return_type() {
    let schema = Switched.plugin_schema();
    let ir = Switched::activation_ir();

    for name in ["echo", "whoami", "ping"] {
        let m = ir.method(name).expect("method is in the IR");
        let s = schema
            .methods
            .iter()
            .find(|s| s.name == name)
            .expect("method is in the schema");

        // The return type says "there is an update item type".
        assert!(
            m.is_streaming(),
            "{name}: the fixture returns `impl Stream<Item = String>`, so the IR \
             must see an update item type"
        );

        // The schema says "the author did not ask for SSE" — and that is the
        // value the HTTP gateway routes on. If this flips, ~154 live methods
        // across plexus-idp, plexus-registry and plexus-substrate change wire
        // format; that is a fleet decision, not a macro cleanup.
        assert!(
            !s.streaming,
            "{name}: no `#[method(streaming)]` is declared, so MethodSchema.streaming \
             must stay false — it is a transport declaration, not a fact about the \
             return type"
        );
    }
}
