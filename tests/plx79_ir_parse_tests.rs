//! PLX-79 (M1·B) acceptance tests: `#[method]` signatures become real
//! `plexus_core::ir::MethodIr` values, and an impl block becomes an
//! `ActivationIr`.
//!
//! These are the half of the parser's contract that cannot be checked inside
//! the proc-macro crate: a `SchemaRef` holds a *resolved* `schemars::Schema` and
//! a callback set comes from `<C as CapabilitySet>::callbacks()`, both of which
//! only exist once rustc has type-checked the emitted expression. So the macro
//! is invoked here, in a crate that can see plexus-core, and the resulting
//! values are asserted field by field. The token-level half is covered by the
//! unit tests in `src/ir_parse/tests.rs`.
//!
//! Run with: `cargo test --test plx79_ir_parse_tests`.

use plexus_core::capability::{
    Capability, CapabilitySet, Client, FsRead, FsWrite, Permission, Terminal,
};
use plexus_core::ir::{ActivationIr, AuthRequirementIr, ChildEdge, HttpMethodIr, SchemaRef};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// --- fixture payload types -------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct Node {
    id: String,
    label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct Update {
    chunk: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct Answer {
    text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct FetchError {
    message: String,
}

/// Every `SchemaRef` in a document, so tests can assert a *global* property
/// over them rather than one field at a time.
fn all_schema_refs(ir: &ActivationIr) -> Vec<&SchemaRef> {
    let mut out = Vec::new();
    for m in &ir.methods {
        out.extend(m.params.iter().map(|p| &p.schema));
        out.extend(m.updates.iter());
        out.extend(m.terminal.iter());
        for c in &m.callbacks {
            out.push(&c.request);
            out.push(&c.response);
        }
    }
    for c in &ir.children {
        if let ChildEdge::Static(child) = c {
            out.extend(all_schema_refs(child));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// The shape matrix
// ---------------------------------------------------------------------------

#[test]
fn unary_result_method() {
    let ir = plexus_macros::method_ir!(
        namespace = "claudecode",
        /// Read one node.
        #[method(http_method = "GET")]
        async fn get(&self, id: String) -> Result<Node, FetchError>;
    )
    .expect("MethodIr builds");

    assert_eq!(ir.name, "get");
    assert_eq!(ir.dotted_id, "claudecode.get");
    assert_eq!(ir.description, "Read one node.");
    assert_eq!(ir.http_method, HttpMethodIr::Get);
    assert_eq!(ir.auth, AuthRequirementIr::Required);

    assert_eq!(ir.params.len(), 1);
    assert_eq!(ir.params[0].name, "id");
    assert_eq!(ir.params[0].schema.type_name(), "String");
    assert!(ir.params[0].required);

    assert!(ir.updates.is_none());
    assert!(!ir.is_streaming());
    let terminal = ir.terminal.as_ref().expect("unary turns carry a terminal");
    assert_eq!(terminal.type_name(), "Node");
    // The schema is *resolved*, not a name: it describes Node's fields.
    let schema = serde_json::to_value(terminal.schema()).unwrap();
    assert!(schema["properties"]["label"].is_object(), "{schema}");

    assert!(ir.callbacks.is_empty());
    assert!(!ir.is_bidirectional());
    assert!(ir.extensions.is_empty());
}

#[test]
fn streaming_method() {
    let ir = plexus_macros::method_ir!(
        namespace = "claudecode",
        #[method]
        async fn chat(&self, prompt: String) -> impl Stream<Item = Update> + Send + 'static;
    )
    .expect("MethodIr builds");

    assert!(ir.is_streaming());
    let updates = ir.updates.as_ref().unwrap();
    assert_eq!(updates.type_name(), "Update");
    let schema = serde_json::to_value(updates.schema()).unwrap();
    assert!(schema["properties"]["chunk"].is_object(), "{schema}");

    // PLX-75's property: a streaming method with no distinct terminal type
    // carries `None`, not a unit SchemaRef.
    assert!(ir.terminal.is_none());
}

#[test]
fn no_unit_schema_ref_is_ever_constructed() {
    // Three unit-shaped turns, one per syntactic form.
    let no_return = plexus_macros::method_ir!(
        namespace = "ns",
        #[method]
        async fn a(&self);
    )
    .unwrap();
    let explicit_unit = plexus_macros::method_ir!(
        namespace = "ns",
        #[method]
        async fn b(&self) -> ();
    )
    .unwrap();
    let unit_in_result = plexus_macros::method_ir!(
        namespace = "ns",
        #[method]
        async fn c(&self) -> Result<(), FetchError>;
    )
    .unwrap();
    let streaming = plexus_macros::method_ir!(
        namespace = "ns",
        #[method]
        async fn d(&self) -> impl Stream<Item = Update>;
    )
    .unwrap();

    for m in [&no_return, &explicit_unit, &unit_in_result, &streaming] {
        assert!(m.terminal.is_none(), "{}: terminal must be None", m.name);
    }
    for m in [&no_return, &explicit_unit, &unit_in_result] {
        assert!(m.updates.is_none(), "{}: updates must be None", m.name);
    }

    // And the property holds globally over a whole document: no SchemaRef
    // anywhere names the unit type. (`SchemaRef::new` would have *rejected*
    // one — the point is that no emitted path even attempts it, which is why
    // the unit-returning methods above build at all.)
    let doc = fixture_document();
    for s in all_schema_refs(&doc) {
        assert!(
            !matches!(s.type_name(), "()" | "unit" | "!"),
            "unit SchemaRef leaked: {}",
            s.type_name()
        );
    }
    assert!(SchemaRef::new("()", schemars::schema_for!(Node)).is_err());
}

#[test]
fn capability_handle_method() {
    let ir = plexus_macros::method_ir!(
        namespace = "acp",
        /// Run a turn that may call back.
        #[method]
        async fn prompt(
            &self,
            text: String,
            client: Client<(Permission, FsRead)>,
        ) -> Result<Answer, FetchError>;
    )
    .expect("MethodIr builds");

    // Derived from the handle type, not hand-listed: compare against the
    // capability module's own descriptors.
    assert_eq!(ir.callbacks, Client::<(Permission, FsRead)>::callbacks());
    assert_eq!(
        ir.callbacks,
        <(Permission, FsRead) as CapabilitySet>::callbacks()
    );
    assert_eq!(
        ir.callbacks.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
        [Permission::NAME, FsRead::NAME],
    );
    assert_eq!(ir.callbacks[0], Permission::descriptor());
    assert_eq!(ir.callbacks[1], FsRead::descriptor());
    assert!(ir.is_bidirectional());

    // The handle is a declaration, not a parameter.
    assert_eq!(
        ir.params.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
        ["text"],
    );
    assert!(ir.params.iter().all(|p| p.schema.type_name() != "Client"));
}

#[test]
fn a_different_capability_set_yields_different_callbacks() {
    let ir = plexus_macros::method_ir!(
        namespace = "acp",
        #[method]
        async fn build(&self, client: &Client<(FsWrite, Terminal)>) -> Result<Answer, FetchError>;
    )
    .unwrap();

    assert_eq!(ir.callbacks, Client::<(FsWrite, Terminal)>::callbacks());
    assert_eq!(
        ir.callbacks.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
        ["fs/write_text_file", "terminal/create"],
    );
    assert!(ir.params.is_empty());
}

#[test]
fn method_without_a_handle_has_no_callbacks() {
    let ir = plexus_macros::method_ir!(
        namespace = "acp",
        #[method]
        async fn plain(&self) -> Result<Answer, FetchError>;
    )
    .unwrap();
    assert!(ir.callbacks.is_empty());
}

#[test]
fn ext_passthrough_and_param_docs() {
    let ir = plexus_macros::method_ir!(
        namespace = "claudecode",
        #[method(
            params(query = "What to search for", limit = "Maximum results"),
            ext(acp_role = "prompt", nostr_kind = 30023, flag = true)
        )]
        async fn search(
            &self,
            query: String,
            limit: Option<u32>,
        ) -> Result<Vec<Node>, FetchError>;
    )
    .unwrap();

    assert_eq!(ir.params[0].description, "What to search for");
    assert_eq!(ir.params[1].description, "Maximum results");
    // `Option<T>` is optionality, not a wrapped schema.
    assert!(ir.params[0].required);
    assert!(!ir.params[1].required);
    assert_eq!(ir.params[1].schema.type_name(), "u32");
    assert_eq!(ir.terminal.as_ref().unwrap().type_name(), "Vec<Node>");

    assert_eq!(ir.extensions.len(), 3);
    assert_eq!(ir.extensions["acp_role"], serde_json::json!("prompt"));
    assert_eq!(ir.extensions["nostr_kind"], serde_json::json!(30023));
    assert_eq!(ir.extensions["flag"], serde_json::json!(true));
}

#[test]
fn auth_parameter_drives_the_auth_requirement() {
    let required = plexus_macros::method_ir!(
        namespace = "ns",
        #[method]
        async fn a(&self, auth: &AuthContext, id: String) -> Result<Node, FetchError>;
    )
    .unwrap();
    assert_eq!(required.auth, AuthRequirementIr::Required);
    assert_eq!(required.params.len(), 1);

    let optional = plexus_macros::method_ir!(
        namespace = "ns",
        #[method]
        async fn b(&self, auth: Option<&AuthContext>) -> Result<Node, FetchError>;
    )
    .unwrap();
    assert_eq!(optional.auth, AuthRequirementIr::Optional);
    assert!(optional.params.is_empty());
}

#[test]
fn every_http_verb_maps_to_the_real_enum() {
    macro_rules! verb {
        ($lit:literal) => {
            plexus_macros::method_ir!(
                namespace = "ns",
                #[method(http_method = $lit)]
                async fn f(&self) -> Result<Node, FetchError>;
            )
            .unwrap()
            .http_method
        };
    }
    assert_eq!(verb!("GET"), HttpMethodIr::Get);
    assert_eq!(verb!("POST"), HttpMethodIr::Post);
    assert_eq!(verb!("PUT"), HttpMethodIr::Put);
    assert_eq!(verb!("DELETE"), HttpMethodIr::Delete);
    assert_eq!(verb!("PATCH"), HttpMethodIr::Patch);
    // Undeclared → the IR's own default, not a String round-trip.
    let default = plexus_macros::method_ir!(
        namespace = "ns",
        #[method]
        async fn g(&self) -> Result<Node, FetchError>;
    )
    .unwrap();
    assert_eq!(default.http_method, HttpMethodIr::default());
}

#[test]
fn deprecation_reaches_the_ir() {
    let ir = plexus_macros::method_ir!(
        namespace = "ns",
        #[method]
        #[deprecated(since = "0.6", note = "use `get`")]
        async fn old_get(&self) -> Result<Node, FetchError>;
    )
    .unwrap();
    let d = ir.deprecation.expect("deprecation carried");
    assert_eq!(d.since, "0.6");
    assert_eq!(d.message, "use `get`");
    assert_eq!(d.removed_in, None);
}

// ---------------------------------------------------------------------------
// Activation assembly
// ---------------------------------------------------------------------------

struct Session;

impl Session {
    /// The crate-internal convention a parent uses to embed a child's subtree.
    /// Build F generates this; here it is written by hand, which is exactly how
    /// a hand-authored child would participate.
    fn __plexus_activation_ir(
    ) -> Result<ActivationIr, plexus_core::ir::SchemaRefError> {
        plexus_macros::activation_ir!(
            namespace = "session",
            version = "1.0.0",
            impl Session {
                #[method]
                async fn chat(&self, prompt: String) -> impl Stream<Item = Update> {
                    todo!()
                }
            }
        )
    }
}

struct Jsexec;

impl Jsexec {
    fn __plexus_activation_ir(
    ) -> Result<ActivationIr, plexus_core::ir::SchemaRefError> {
        plexus_macros::activation_ir!(
            namespace = "jsexec",
            version = "1.0.0",
            impl Jsexec {
                #[method]
                async fn eval(&self, src: String) -> Result<Answer, FetchError> {
                    todo!()
                }
            }
        )
    }
}

struct ClaudeCode;

fn fixture_document() -> ActivationIr {
    plexus_macros::activation_ir!(
        namespace = "claudecode",
        version = "2.0.0",
        backend_name = "substrate",
        respond_method = "respond",
        /// Drive Claude Code sessions.
        impl ClaudeCode {
            /// List sessions.
            #[method]
            async fn list(&self) -> Result<Vec<Node>, FetchError> {
                todo!()
            }

            /// Fire and forget.
            #[method]
            async fn touch(&self) {
                todo!()
            }

            /// A turn with callbacks.
            #[method]
            async fn prompt(
                &self,
                text: String,
                client: Client<(Permission, FsRead)>,
            ) -> impl Stream<Item = Update> {
                todo!()
            }

            /// Not annotated — not in the IR.
            async fn helper(&self) -> u8 {
                todo!()
            }

            #[child]
            fn embedded(&self) -> Jsexec {
                todo!()
            }

            #[child(list = "list")]
            fn session(&self) -> Session {
                todo!()
            }

            #[child]
            fn plugins(&self, name: &str) -> Option<Jsexec> {
                todo!()
            }
        }
    )
    .expect("ActivationIr builds")
}

#[test]
fn activation_assembles_methods_and_child_edges() {
    let ir = fixture_document();

    assert_eq!(ir.namespace, "claudecode");
    assert_eq!(ir.version, "2.0.0");
    assert_eq!(ir.backend_name.as_deref(), Some("substrate"));
    assert_eq!(ir.respond_method.as_deref(), Some("respond"));
    assert_eq!(ir.description, "Drive Claude Code sessions.");

    assert_eq!(
        ir.methods.iter().map(|m| m.dotted_id.as_str()).collect::<Vec<_>>(),
        ["claudecode.list", "claudecode.touch", "claudecode.prompt"],
    );
    assert!(ir.method("helper").is_none());

    // Hashes are computed for the whole tree.
    assert!(ir.ir_hash.is_some());
    assert!(!ir.hash.is_empty());
    assert!(ir.methods.iter().all(|m| !m.hash.is_empty()));

    match ir.child("embedded").expect("static edge") {
        ChildEdge::Static(child) => {
            assert_eq!(child.namespace, "embedded");
            assert_eq!(child.method("eval").unwrap().dotted_id, "jsexec.eval");
            assert!(!child.hash.is_empty());
            // A `Static` child is a node, not a document.
            assert!(child.ir_hash.is_none());
        }
        other => panic!("expected Static, got {other:?}"),
    }

    match ir.child("session").expect("indexed edge") {
        ChildEdge::Indexed {
            list_method,
            search_method,
            id_field,
            path_template,
            template,
            ..
        } => {
            assert_eq!(list_method, "claudecode.list");
            assert!(search_method.is_none());
            assert_eq!(id_field, "name");
            assert_eq!(path_template, "session/{id}");
            assert_eq!(template.namespace, "session");
            assert!(template.method("chat").unwrap().is_streaming());
        }
        other => panic!("expected Indexed, got {other:?}"),
    }

    match ir.child("plugins").expect("dynamic edge") {
        ChildEdge::Dynamic { namespace, .. } => assert_eq!(namespace, "plugins"),
        other => panic!("expected Dynamic, got {other:?}"),
    }
}

#[test]
fn the_document_round_trips_and_its_hashes_are_merkle() {
    let ir = fixture_document();

    // M1-T1: lossless serialization.
    let json = serde_json::to_string(&ir).unwrap();
    let back: ActivationIr = serde_json::from_str(&json).unwrap();
    assert_eq!(back, ir);

    // M1-T2: mutating a method changes its hash, its ancestors', and the
    // document digest — and nothing else.
    let mut mutated = ir.clone();
    mutated.methods[0].description = "changed".to_string();
    mutated.recompute_hashes();
    assert_ne!(mutated.methods[0].hash, ir.methods[0].hash);
    assert_eq!(mutated.methods[1].hash, ir.methods[1].hash);
    assert_ne!(mutated.hash, ir.hash);
    assert_ne!(mutated.ir_hash, ir.ir_hash);
}
