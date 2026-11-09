//! IR-3 acceptance tests: plexus-macros emits `MethodRole` and
//! `DeprecationInfo` on generated `MethodSchema` entries.
//!
//! Coverage map (acceptance criteria from `plans/IR/IR-3.md`):
//!
//! | AC | Verified by                                            |
//! |----|--------------------------------------------------------|
//! | 2  | every test in this file runs via `cargo test`          |
//! | 3  | `role_tagging_covers_rpc_static_and_dynamic`           |
//! | 5  | `deprecation_with_since_note_and_removed_in_companion` |
//! | 7  | `legacy_children_syntax_still_emits_populated_schema`  |
//!
//! AC 1 is checked by `cargo build -p plexus-macros`; ACs 4 and 6 are
//! trybuild fixtures in `tests/compile/` (see `ir3_compile_tests.rs`); AC 8
//! is checked by `cargo build --workspace` in plexus-substrate.
//!
//! Run with: `cargo test --test ir3_role_and_deprecation_tests`.

use async_stream::stream;
use futures::stream::Stream;
use plexus_core::plexus::{DeprecationInfo, MethodRole};

// ---------------------------------------------------------------------------
// Shared leaf child used by the fixtures below. Declared with the macro so
// it's a valid `ChildRouter` destination.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct Leaf;

#[plexus_macros::activation(
    namespace = "leaf",
    version = "1.0.0",
    crate_path = "plexus_core"
)]
impl Leaf {
    /// Always-available health check.
    #[plexus_macros::method]
    async fn ping(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "pong".into(); }
    }
}

// ---------------------------------------------------------------------------
// AC #3: role tagging covers all three roles in a single impl block.
//
// The fixture hand-writes `plugin_children(&self) -> Vec<ChildSummary>` to
// suppress the macro's `synthesize_plugin_children` path so ALL child
// methods (static + dynamic) surface as `MethodSchema` entries — otherwise
// plexus-core's `validate_no_collisions` panics when a static child name
// appears on both the methods list and the children list. IR-4 drops the
// children side-channel and removes this interaction.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct RoleCoverageHub {
    mercury: Leaf,
}

#[plexus_macros::activation(
    namespace = "role_coverage",
    version = "1.0.0",
    crate_path = "plexus_core"
)]
impl RoleCoverageHub {
    /// RPC method — role = Rpc.
    #[plexus_macros::method]
    async fn ping(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "pong".into(); }
    }

    /// Static child — role = StaticChild.
    #[plexus_macros::child]
    fn mercury(&self) -> Leaf {
        self.mercury.clone()
    }

    /// Dynamic child with list/search opt-in — role = DynamicChild { list, search }.
    #[plexus_macros::child(list = list_fn, search = search_fn)]
    fn planet(&self, _name: &str) -> Option<Leaf> {
        Some(Leaf)
    }

    fn list_fn(&self) -> impl Stream<Item = String> + Send + '_ {
        stream! { yield "jupiter".to_string(); }
    }

    fn search_fn(&self, _query: &str) -> impl Stream<Item = String> + Send + '_ {
        stream! { yield "jupiter".to_string(); }
    }

    /// Hand-written `plugin_children` suppresses the macro's synthesis so the
    /// static `#[child]` method can freely surface as a `StaticChild`-roled
    /// `MethodSchema` entry without colliding with a `ChildSummary` of the
    /// same name. IR-4 drops this interaction.
    pub fn plugin_children(&self) -> Vec<plexus_core::plexus::ChildSummary> {
        Vec::new()
    }
}

fn lookup_role(schemas: &[plexus_core::plexus::MethodSchema], name: &str) -> MethodRole {
    let role = schemas
        .iter()
        .find(|m| m.name == name)
        .unwrap_or_else(|| panic!("method `{}` not found in schemas", name))
        .role
        .clone();
    // NEST-5: a dynamic child's role carries its slot type too; these rows
    // are about list/search, so the type is set aside here (nest5 tests it).
    match role {
        MethodRole::DynamicChild { list_method, search_method, .. } => {
            MethodRole::DynamicChild { list_method, search_method, child: None }
        }
        other => other,
    }
}

#[test]
fn role_tagging_covers_rpc_static_and_dynamic() {
    let schemas = RoleCoverageHubMethod::method_schemas();

    assert_eq!(lookup_role(&schemas, "ping"), MethodRole::Rpc);
    assert_eq!(lookup_role(&schemas, "mercury"), MethodRole::StaticChild);
    assert_eq!(
        lookup_role(&schemas, "planet"),
        MethodRole::DynamicChild {
            list_method: Some("list_fn".into()),
            search_method: Some("search_fn".into()),
            child: None,
        },
    );
}

#[test]
fn role_tagging_bare_dynamic_emits_none_list_and_search() {
    // Companion coverage: a bare `#[child]` with a `name: &str` arg and no
    // list/search args emits `DynamicChild { list_method: None, search_method: None }`.
    #[derive(Clone)]
    struct BareDyn;

    #[plexus_macros::activation(
        namespace = "bare_dyn",
        version = "1.0.0",
        crate_path = "plexus_core"
    )]
    impl BareDyn {
        #[plexus_macros::method]
        async fn ping(&self) -> impl Stream<Item = String> + Send + 'static {
            stream! { yield "pong".into(); }
        }

        #[plexus_macros::child]
        fn any(&self, _name: &str) -> Option<Leaf> {
            Some(Leaf)
        }
    }

    let schemas = BareDynMethod::method_schemas();
    assert_eq!(
        lookup_role(&schemas, "any"),
        MethodRole::DynamicChild {
            list_method: None,
            search_method: None,
            child: None,
        },
    );
}

#[test]
fn role_tagging_list_only_and_search_only_variants() {
    // Coverage for the middle rows of the MethodRole table:
    //   #[child(list = X)]                 → DynamicChild { Some("X"), None }
    //   #[child(search = Y)]               → DynamicChild { None, Some("Y") }
    #[derive(Clone)]
    struct MixedDyn;

    #[plexus_macros::activation(
        namespace = "mixed_dyn",
        version = "1.0.0",
        crate_path = "plexus_core"
    )]
    impl MixedDyn {
        #[plexus_macros::method]
        async fn ping(&self) -> impl Stream<Item = String> + Send + 'static {
            stream! { yield "pong".into(); }
        }

        #[plexus_macros::child(list = only_list)]
        fn list_only(&self, _name: &str) -> Option<Leaf> {
            Some(Leaf)
        }

        fn only_list(&self) -> impl Stream<Item = String> + Send + '_ {
            stream! { yield "x".to_string(); }
        }
    }

    let schemas = MixedDynMethod::method_schemas();
    assert_eq!(
        lookup_role(&schemas, "list_only"),
        MethodRole::DynamicChild {
            list_method: Some("only_list".into()),
            search_method: None,
            child: None,
        },
    );

    #[derive(Clone)]
    struct SearchOnlyDyn;

    #[plexus_macros::activation(
        namespace = "search_only_dyn",
        version = "1.0.0",
        crate_path = "plexus_core"
    )]
    impl SearchOnlyDyn {
        #[plexus_macros::method]
        async fn ping(&self) -> impl Stream<Item = String> + Send + 'static {
            stream! { yield "pong".into(); }
        }

        #[plexus_macros::child(search = only_search)]
        fn search_only(&self, _name: &str) -> Option<Leaf> {
            Some(Leaf)
        }

        fn only_search(&self, _q: &str) -> impl Stream<Item = String> + Send + '_ {
            stream! { yield "x".to_string(); }
        }
    }

    let schemas2 = SearchOnlyDynMethod::method_schemas();
    assert_eq!(
        lookup_role(&schemas2, "search_only"),
        MethodRole::DynamicChild {
            list_method: None,
            search_method: Some("only_search".into()),
            child: None,
        },
    );
}

// ---------------------------------------------------------------------------
// AC #2 (default behavior): activations without any `#[deprecated]` attribute
// emit `deprecation: None` for every method.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct NoDeprecations;

#[plexus_macros::activation(
    namespace = "no_deprecations",
    version = "1.0.0",
    crate_path = "plexus_core"
)]
impl NoDeprecations {
    /// Plain RPC method.
    #[plexus_macros::method]
    async fn one(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "1".into(); }
    }

    /// Another plain RPC method.
    #[plexus_macros::method]
    async fn two(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "2".into(); }
    }
}

#[test]
fn methods_without_deprecated_emit_none_deprecation() {
    let schemas = NoDeprecationsMethod::method_schemas();
    for m in &schemas {
        assert!(
            m.deprecation.is_none(),
            "method `{}` unexpectedly has deprecation = {:?}",
            m.name,
            m.deprecation
        );
    }
}

// ---------------------------------------------------------------------------
// AC #5: `#[deprecated(since = "0.5", note = "use bar")]` +
// `#[plexus_macros::removed_in("0.6")]` → `Some(DeprecationInfo { "0.5",
// "0.6", "use bar" })`.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct DeprecatedHub;

#[allow(deprecated)]
#[plexus_macros::activation(
    namespace = "deprecated_hub",
    version = "1.0.0",
    crate_path = "plexus_core"
)]
impl DeprecatedHub {
    /// Not deprecated — should emit `deprecation: None`.
    #[plexus_macros::method]
    async fn fresh(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "fresh".into(); }
    }

    /// Deprecated method — expect DeprecationInfo with all three fields
    /// populated, sourced from `since` / `note` (rustc keys) and the companion
    /// `removed_in` (plexus-macros companion).
    #[deprecated(since = "0.5", note = "use bar")]
    #[plexus_macros::removed_in("0.6")]
    #[plexus_macros::method]
    async fn old(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "old".into(); }
    }
}

#[test]
fn deprecation_with_since_note_and_removed_in_companion() {
    let schemas = DeprecatedHubMethod::method_schemas();

    let fresh = schemas.iter().find(|m| m.name == "fresh").unwrap();
    assert!(fresh.deprecation.is_none());

    let old = schemas.iter().find(|m| m.name == "old").unwrap();
    assert_eq!(
        old.deprecation,
        Some(DeprecationInfo {
            since: "0.5".into(),
            removed_in: "0.6".into(),
            message: "use bar".into(),
        }),
    );
}

// ---------------------------------------------------------------------------
// AC (Context table): `#[deprecated]` without the companion attribute defaults
// `removed_in` to the literal string `"unspecified"`.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct DeprecatedNoCompanion;

#[allow(deprecated)]
#[plexus_macros::activation(
    namespace = "dep_no_companion",
    version = "1.0.0",
    crate_path = "plexus_core"
)]
impl DeprecatedNoCompanion {
    #[deprecated(since = "0.5", note = "use bar")]
    #[plexus_macros::method]
    async fn old(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "old".into(); }
    }
}

#[test]
fn deprecation_without_companion_uses_unspecified_removed_in() {
    let schemas = DeprecatedNoCompanionMethod::method_schemas();
    let old = schemas.iter().find(|m| m.name == "old").unwrap();
    assert_eq!(
        old.deprecation,
        Some(DeprecationInfo {
            since: "0.5".into(),
            removed_in: "unspecified".into(),
            message: "use bar".into(),
        }),
    );
}

// ---------------------------------------------------------------------------
// AC (prompt item 4): `removed_in` written directly inside `#[deprecated(...)]`.
//
// Current stable rustc emits a hard error (`E0541: unknown meta item
// 'removed_in'`) for unknown keys in `#[deprecated]`, so a positive fixture
// exercising that form cannot compile. The macro's parser, however, is
// written to read `removed_in` from inside `#[deprecated]` when present —
// behavior that's exercised by the unit test
// `parse_deprecation_attrs::inline_removed_in_key_is_read_from_deprecated`
// in src/parse.rs, which drives the parser directly on synthetic
// `syn::Attribute` inputs without involving rustc's attribute checker.
// ---------------------------------------------------------------------------


// ---------------------------------------------------------------------------
// AC #7: legacy activation syntax (`hub` flag + hand-written `plugin_children`)
// continues to emit a populated `children: Vec<ChildSummary>` AND the
// `methods` list carries method roles per `#[method]`/`#[child]` use. Both
// surfaces coexist. IR-3 is additive — it does not remove the legacy path.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct LegacyHubSchema;

#[plexus_macros::activation(
    namespace = "legacy_hub_schema",
    version = "1.0.0",
    hub,
    crate_path = "plexus_core"
)]
impl LegacyHubSchema {
    #[plexus_macros::method]
    async fn first(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "1".into(); }
    }

    #[plexus_macros::method]
    async fn second(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "2".into(); }
    }

    /// Hand-written `plugin_children` — legacy hub pattern.
    pub fn plugin_children(&self) -> Vec<plexus_core::plexus::ChildSummary> {
        vec![plexus_core::plexus::ChildSummary {
            namespace: "mercury".into(),
            description: "legacy".into(),
            hash: String::new(),
        }]
    }
}

// Legacy hubs with the explicit `hub` flag need a hand-written `ChildRouter`.
#[async_trait::async_trait]
impl plexus_core::plexus::ChildRouter for LegacyHubSchema {
    fn router_namespace(&self) -> &str {
        "legacy_hub_schema"
    }

    async fn router_call(
        &self,
        method: &str,
        params: serde_json::Value,
        auth: Option<&plexus_core::plexus::AuthContext>,
        raw_ctx: Option<&plexus_core::request::RawRequestContext>,
    ) -> Result<plexus_core::plexus::PlexusStream, plexus_core::plexus::PlexusError> {
        <Self as plexus_core::plexus::Activation>::call(self, method, params, auth, raw_ctx).await
    }

    async fn get_child(
        &self,
        _name: &str,
    ) -> Option<Box<dyn plexus_core::plexus::ChildRouter>> {
        None
    }
}

#[test]
fn legacy_children_syntax_still_emits_populated_schema() {
    use plexus_core::plexus::Activation;

    let hub = LegacyHubSchema;
    let schema = hub.plugin_schema();

    // PluginSchema.children populated from the hand-written plugin_children().
    let children = schema.children.as_ref().expect("legacy children populated");
    assert_eq!(children.len(), 1);
    assert_eq!(children[0].namespace, "mercury");

    // methods list carries the two #[method]s with Rpc role.
    let mut by_name: std::collections::HashMap<&str, &plexus_core::plexus::MethodRole> =
        std::collections::HashMap::new();
    for m in &schema.methods {
        by_name.insert(m.name.as_str(), &m.role);
    }
    assert_eq!(by_name.get("first"), Some(&&MethodRole::Rpc));
    assert_eq!(by_name.get("second"), Some(&&MethodRole::Rpc));
}
