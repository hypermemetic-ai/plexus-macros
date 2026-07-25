//! Unit tests for the token half of the parser.
//!
//! These assert on [`MethodIrSpec`] — everything the macro can decide without
//! rustc. The other half (that the emitted expression really evaluates to the
//! right `MethodIr`, with real schemas and real capability descriptors) is
//! asserted in `tests/plx79_ir_parse_tests.rs`, which can depend on plexus-core.

use quote::quote;
use syn::{parse_quote, ItemImpl, TraitItemFn};

use super::signature::{is_unit, type_name_of};
use super::spec::{ActivationIrSpec, AuthRequirementSpec, ChildEdgeSpec, HttpMethodIrSpec};
use super::MethodIrSpec;

fn spec(func: TraitItemFn) -> MethodIrSpec {
    MethodIrSpec::from_signature("ns", &func).expect("signature should parse")
}

fn err(func: TraitItemFn) -> String {
    MethodIrSpec::from_signature("ns", &func)
        .expect_err("signature should be rejected")
        .to_string()
}

#[test]
fn unary_result_gives_a_terminal_and_no_updates() {
    let s = spec(parse_quote! {
        /// Read a node.
        #[method]
        async fn get(&self, id: String) -> Result<Node, Error>;
    });

    assert_eq!(s.name, "get");
    assert_eq!(s.dotted_id, "ns.get");
    assert_eq!(s.description, "Read a node.");
    assert_eq!(s.params.len(), 1);
    assert_eq!(s.params[0].name, "id");
    assert!(s.params[0].required);
    assert!(s.updates.is_none());
    assert_eq!(type_name_of(s.terminal.as_ref().unwrap()), "Node");
    assert!(s.capability_set.is_none());
    assert_eq!(s.http_method, HttpMethodIrSpec::Post);
    assert_eq!(s.auth, AuthRequirementSpec::Required);
}

#[test]
fn streaming_gives_updates_and_no_terminal() {
    let s = spec(parse_quote! {
        #[method]
        async fn chat(&self) -> impl Stream<Item = Update> + Send + 'static;
    });

    assert_eq!(type_name_of(s.updates.as_ref().unwrap()), "Update");
    // PLX-75: a streaming method has no *distinct* terminal type, and the
    // absence is spelled `None` rather than a unit SchemaRef.
    assert!(s.terminal.is_none());
}

#[test]
fn streaming_inside_result_is_still_streaming() {
    let s = spec(parse_quote! {
        #[method]
        async fn chat(&self) -> Result<impl Stream<Item = Update>, Error>;
    });
    assert_eq!(type_name_of(s.updates.as_ref().unwrap()), "Update");
    assert!(s.terminal.is_none());
}

#[test]
fn unit_returns_produce_no_schema_ref_at_all() {
    for f in [
        parse_quote! { #[method] async fn f(&self); },
        parse_quote! { #[method] async fn f(&self) -> (); },
        parse_quote! { #[method] async fn f(&self) -> Result<(), Error>; },
    ] {
        let s: MethodIrSpec = spec(f);
        assert!(s.updates.is_none(), "no updates for a unit turn");
        assert!(
            s.terminal.is_none(),
            "a unit terminal is `None`, never a unit SchemaRef"
        );
    }
}

#[test]
fn option_params_are_optional_over_the_inner_type() {
    let s = spec(parse_quote! {
        #[method(params(limit = "How many"))]
        async fn list(&self, limit: Option<u32>) -> Result<Vec<Node>, Error>;
    });
    assert_eq!(s.params.len(), 1);
    assert!(!s.params[0].required);
    assert_eq!(type_name_of(&s.params[0].ty), "u32");
    assert_eq!(s.params[0].description, "How many");
}

#[test]
fn the_capability_handle_is_not_a_parameter() {
    let s = spec(parse_quote! {
        #[method]
        async fn prompt(
            &self,
            text: String,
            client: Client<(Permission, FsRead)>,
        ) -> Result<Answer, Error>;
    });

    assert_eq!(
        s.params.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
        ["text"],
        "the handle declares callbacks; it is not caller-supplied"
    );
    assert_eq!(
        type_name_of(s.capability_set.as_ref().unwrap()),
        "(Permission,FsRead)"
    );
}

#[test]
fn a_referenced_handle_is_recognized_too() {
    let s = spec(parse_quote! {
        #[method]
        async fn prompt(&self, client: &Client<(FsWrite,)>) -> Result<Answer, Error>;
    });
    assert!(s.params.is_empty());
    assert_eq!(type_name_of(s.capability_set.as_ref().unwrap()), "(FsWrite,)");
}

#[test]
fn no_handle_means_no_callbacks() {
    let s = spec(parse_quote! {
        #[method]
        async fn plain(&self) -> Result<Answer, Error>;
    });
    assert!(s.capability_set.is_none());
}

#[test]
fn a_handle_without_its_set_is_an_error() {
    let message = err(parse_quote! {
        #[method]
        async fn prompt(&self, client: Client) -> Result<Answer, Error>;
    });
    assert!(
        message.contains("capability set"),
        "error should demand the set: {message}"
    );
}

#[test]
fn two_handles_are_an_error() {
    let message = err(parse_quote! {
        #[method]
        async fn prompt(&self, a: Client<(FsRead,)>, b: Client<(FsWrite,)>) -> Result<A, E>;
    });
    assert!(message.contains("at most one capability handle"), "{message}");
}

#[test]
fn auth_parameters_set_the_requirement_and_are_not_parameters() {
    let required = spec(parse_quote! {
        #[method]
        async fn f(&self, auth: &AuthContext, id: String) -> Result<A, E>;
    });
    assert_eq!(required.auth, AuthRequirementSpec::Required);
    assert_eq!(required.params.len(), 1);

    let optional = spec(parse_quote! {
        #[method]
        async fn f(&self, auth: Option<&AuthContext>) -> Result<A, E>;
    });
    assert_eq!(optional.auth, AuthRequirementSpec::Optional);
    assert!(optional.params.is_empty());
}

#[test]
fn http_method_is_the_real_enum_with_no_string_round_trip() {
    let s = spec(parse_quote! {
        #[method(http_method = "get")]
        async fn f(&self) -> Result<A, E>;
    });
    assert_eq!(s.http_method, HttpMethodIrSpec::Get);
}

#[test]
fn an_invalid_http_verb_is_rejected_at_the_literal() {
    let message = err(parse_quote! {
        #[method(http_method = "FETCH")]
        async fn f(&self) -> Result<A, E>;
    });
    assert!(message.contains("invalid HTTP method"), "{message}");
}

#[test]
fn ext_pairs_pass_through_the_existing_plx71_parser() {
    let s = spec(parse_quote! {
        #[method(ext(nostr_kind = 30023, label = "x", flag = true))]
        async fn f(&self) -> Result<A, E>;
    });
    assert_eq!(
        s.extensions,
        vec![
            ("nostr_kind".to_string(), serde_json::json!(30023)),
            ("label".to_string(), serde_json::json!("x")),
            ("flag".to_string(), serde_json::json!(true)),
        ]
    );
}

#[test]
fn duplicate_ext_keys_still_error_because_the_parser_is_shared() {
    let message = err(parse_quote! {
        #[method(ext(k = 1, k = 2))]
        async fn f(&self) -> Result<A, E>;
    });
    assert!(message.contains("duplicate ext key `k`"), "{message}");
}

#[test]
fn unknown_attribute_keys_are_errors_naming_the_key() {
    let message = err(parse_quote! {
        #[method(unknwon = 1)]
        async fn f(&self) -> Result<A, E>;
    });
    assert!(message.contains("unknwon"), "the key must be named: {message}");
    assert!(message.contains("ext("), "the escape hatch must be named: {message}");
}

#[test]
fn removed_surface_keys_point_at_their_replacement() {
    for (key, needle) in [
        ("bidirectional", "capability handle"),
        ("streaming", "return type"),
        ("scope = \"a.b\"", "MethodIr::auth"),
        ("public", "MethodIr::auth"),
    ] {
        let attr: proc_macro2::TokenStream = key.parse().unwrap();
        let func: TraitItemFn = syn::parse2(quote! {
            #[method(#attr)]
            async fn f(&self) -> Result<A, E>;
        })
        .unwrap();
        let message = err(func);
        assert!(message.contains(needle), "for `{key}`: {message}");
    }
}

#[test]
fn explicit_name_wins_and_dotted_id_follows_it() {
    let s = spec(parse_quote! {
        #[method(name = "renamed", description = "Explicit.")]
        async fn original(&self) -> Result<A, E>;
    });
    assert_eq!(s.name, "renamed");
    assert_eq!(s.dotted_id, "ns.renamed");
    assert_eq!(s.description, "Explicit.");
}

#[test]
fn deprecation_is_carried_over() {
    let s = spec(parse_quote! {
        #[method]
        #[deprecated(since = "0.6", note = "use `get`")]
        async fn old_get(&self) -> Result<A, E>;
    });
    let d = s.deprecation.expect("deprecation parsed");
    assert_eq!(d.since, "0.6");
    assert_eq!(d.message, "use `get`");
}

#[test]
fn is_unit_recognizes_the_unit_forms() {
    assert!(is_unit(&parse_quote!(())));
    assert!(is_unit(&parse_quote!(!)));
    assert!(!is_unit(&parse_quote!(u8)));
}

// ---------------------------------------------------------------------------
// Activation assembly
// ---------------------------------------------------------------------------

fn activation(item: ItemImpl) -> ActivationIrSpec {
    ActivationIrSpec::from_impl("root", "1.2.3", None, None, None, &item)
        .expect("impl block should parse")
}

#[test]
fn an_impl_block_yields_its_methods_and_child_edges() {
    let a = activation(parse_quote! {
        impl Root {
            #[method]
            async fn alpha(&self) -> Result<A, E> { todo!() }

            #[method]
            async fn beta(&self) -> impl Stream<Item = B> { todo!() }

            /// Not annotated — invisible to the IR.
            async fn helper(&self) -> u8 { todo!() }

            #[child]
            fn statics(&self) -> StaticChild { todo!() }

            #[child(list = "alpha")]
            fn indexed(&self) -> IndexedChild { todo!() }

            #[child]
            fn dynamics(&self, name: &str) -> Option<DynChild> { todo!() }
        }
    });

    assert_eq!(a.namespace, "root");
    assert_eq!(a.version, "1.2.3");
    assert_eq!(
        a.methods.iter().map(|m| m.dotted_id.as_str()).collect::<Vec<_>>(),
        ["root.alpha", "root.beta"]
    );
    assert_eq!(a.children.len(), 3);

    assert!(matches!(&a.children[0], ChildEdgeSpec::Static { namespace, .. } if namespace == "statics"));
    match &a.children[1] {
        ChildEdgeSpec::Indexed {
            namespace,
            list_method,
            path_template,
            id_field,
            search_method,
            ..
        } => {
            assert_eq!(namespace, "indexed");
            assert_eq!(list_method, "root.alpha");
            assert_eq!(path_template, "indexed/{id}");
            assert_eq!(id_field, "name");
            assert!(search_method.is_none());
        }
        other => panic!("expected an indexed edge, got {other:?}"),
    }
    assert!(matches!(&a.children[2], ChildEdgeSpec::Dynamic { namespace, .. } if namespace == "dynamics"));
}
