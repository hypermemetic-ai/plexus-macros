//! Emission: token-level spec → an expression that evaluates to the real IR.
//!
//! This is the half of the parser that only rustc can finish. Three kinds of
//! value are emitted rather than computed:
//!
//! - **schemas** — `SchemaRef::new("T", schemars::schema_for!(T))`, because a
//!   JSON Schema is a property of a *resolved* type;
//! - **callbacks** — `<C as CapabilitySet>::callbacks()`, with `C` the author's
//!   own tokens, so the descriptors come from plexus-core's sealed markers and
//!   this crate holds no capability knowledge whatsoever;
//! - **child subtrees** — `<ChildTy>::__plexus_activation_ir()`, since a child's
//!   IR belongs to the child's own impl block (build F generates that
//!   associated function; a fixture can also write it by hand).
//!
//! Every emitted expression is *fallible*: `SchemaRef::new` returns a `Result`
//! (PLX-75), so the `?` operator appears throughout and the whole thing is
//! wrapped in a `Result`-returning closure by the entry points in
//! [`super`]. Nothing is `.expect()`-ed — an unrepresentable type must surface
//! as a value, not a panic in generated code.

use proc_macro2::{Span, TokenStream};
use proc_macro_crate::{crate_name, FoundCrate};
use quote::quote;
use syn::Type;

use super::signature::type_name_of;
use super::spec::{
    ActivationIrSpec, AuthRequirementSpec, ChildEdgeSpec, HttpMethodIrSpec, MethodIrSpec,
    ParamIrSpec,
};
use crate::codegen::{derive_core_path, derive_serde_json_path};
use crate::parse::ParsedDeprecation;

/// The associated function a child activation exposes so its parent can embed
/// its subtree. Crate-internal convention, not a plexus-core API.
pub(crate) const CHILD_IR_FN: &str = "__plexus_activation_ir";

/// Absolute paths to the crates the emitted expression names.
pub(crate) struct Paths {
    pub(crate) core: syn::Path,
    pub(crate) schemars: syn::Path,
    pub(crate) serde_json: syn::Path,
}

impl Paths {
    pub(crate) fn resolve() -> Self {
        let span = Span::call_site();
        Self {
            core: derive_core_path(span),
            schemars: schemars_path(span),
            serde_json: derive_serde_json_path(span),
        }
    }
}

/// Same additive contract as [`derive_core_path`]: respect a rename, fall back
/// to the `plexus-rpc` umbrella's re-export, else the extern prelude.
fn schemars_path(span: Span) -> syn::Path {
    match crate_name("schemars") {
        Ok(FoundCrate::Name(name)) => {
            let ident = syn::Ident::new(&name, span);
            syn::parse_quote!(::#ident)
        }
        Ok(FoundCrate::Itself) => syn::parse_quote!(::schemars),
        Err(_) => match crate::codegen::umbrella_crate(span) {
            Some(umbrella) => syn::parse_quote!(::#umbrella::__deps::schemars),
            None => syn::parse_quote!(::schemars),
        },
    }
}

/// `SchemaRef::new(<canonical name>, schema_for!(T))?`
///
/// Only ever called for a type the parser already proved non-unit, which is why
/// no code path in this crate can construct a unit `SchemaRef`.
fn schema_ref(paths: &Paths, ty: &Type) -> TokenStream {
    let core = &paths.core;
    let schemars = &paths.schemars;
    let name = type_name_of(ty);
    quote! {
        #core::ir::SchemaRef::new(#name, #schemars::schema_for!(#ty))?
    }
}

fn param_expr(paths: &Paths, p: &ParamIrSpec) -> TokenStream {
    let core = &paths.core;
    let name = &p.name;
    let schema = schema_ref(paths, &p.ty);
    let description = (!p.description.is_empty()).then(|| {
        let d = &p.description;
        quote! { .with_description(#d) }
    });
    let optional = (!p.required).then(|| quote! { .optional() });
    quote! { #core::ir::ParamIr::new(#name, #schema) #description #optional }
}

/// Token-level enum → real enum. Exhaustive, no `String`, no fallback arm.
fn http_method_tokens(paths: &Paths, m: HttpMethodIrSpec) -> TokenStream {
    let core = &paths.core;
    let variant = match m {
        HttpMethodIrSpec::Get => quote!(Get),
        HttpMethodIrSpec::Post => quote!(Post),
        HttpMethodIrSpec::Put => quote!(Put),
        HttpMethodIrSpec::Delete => quote!(Delete),
        HttpMethodIrSpec::Patch => quote!(Patch),
    };
    quote! { #core::ir::HttpMethodIr::#variant }
}

fn auth_tokens(paths: &Paths, a: AuthRequirementSpec) -> TokenStream {
    let core = &paths.core;
    let variant = match a {
        AuthRequirementSpec::Public => quote!(Public),
        AuthRequirementSpec::Optional => quote!(Optional),
        AuthRequirementSpec::Required => quote!(Required),
    };
    quote! { #core::ir::AuthRequirementIr::#variant }
}

fn deprecation_tokens(paths: &Paths, d: &ParsedDeprecation) -> TokenStream {
    let core = &paths.core;
    let since = &d.since;
    let message = &d.message;
    let removed_in = (d.removed_in != crate::parse::DEPRECATION_REMOVED_IN_UNSPECIFIED).then(|| {
        let r = &d.removed_in;
        quote! { .removed_in(#r) }
    });
    quote! { #core::ir::DeprecationIr::new(#since, #message) #removed_in }
}

/// A PLX-71 `ext(...)` value rebuilt as a `serde_json::Value`.
fn ext_value_tokens(paths: &Paths, v: &serde_json::Value) -> TokenStream {
    let sj = &paths.serde_json;
    match v {
        serde_json::Value::String(s) => quote! { #sj::Value::String(#s.to_string()) },
        serde_json::Value::Bool(b) => quote! { #sj::Value::Bool(#b) },
        serde_json::Value::Number(n) => {
            if let Some(u) = n.as_u64() {
                let lit = proc_macro2::Literal::u64_unsuffixed(u);
                quote! { #sj::Value::Number(#sj::Number::from(#lit as u64)) }
            } else if let Some(i) = n.as_i64() {
                let lit = proc_macro2::Literal::i64_unsuffixed(i);
                quote! { #sj::Value::Number(#sj::Number::from(#lit as i64)) }
            } else {
                // The `ext(...)` grammar accepts string / integer / bool
                // literals only, so this is unreachable through the parser; it
                // stays correct rather than lossy if the grammar ever widens.
                let json = v.to_string();
                quote! {
                    <#sj::Value as ::core::str::FromStr>::from_str(#json)
                        .unwrap_or(#sj::Value::Null)
                }
            }
        }
        other => {
            let json = other.to_string();
            quote! {
                <#sj::Value as ::core::str::FromStr>::from_str(#json)
                    .unwrap_or(#sj::Value::Null)
            }
        }
    }
}

/// The `MethodIr` expression. Uses `?`, so it is only valid inside a
/// `Result`-returning scope.
pub(crate) fn method_expr(paths: &Paths, spec: &MethodIrSpec) -> TokenStream {
    let core = &paths.core;
    let name = &spec.name;
    let dotted = &spec.dotted_id;

    let description = (!spec.description.is_empty()).then(|| {
        let d = &spec.description;
        quote! { .with_description(#d) }
    });

    let params = spec.params.iter().map(|p| {
        let e = param_expr(paths, p);
        quote! { .with_param(#e) }
    });

    // RFC 002 §6.9 — content, not a parameter. It is emitted as its own
    // `request_context` field and never appended to `params`, which is the
    // parser-side half of §6.9.5's "MUST NOT merge into the parameters".
    let request_context = spec.request_context.as_ref().map(|t| {
        let s = schema_ref(paths, t);
        quote! { .with_request_context(#s) }
    });

    let updates = spec.updates.as_ref().map(|t| {
        let s = schema_ref(paths, t);
        quote! { .with_updates(#s) }
    });
    let terminal = spec.terminal.as_ref().map(|t| {
        let s = schema_ref(paths, t);
        quote! { .with_terminal(#s) }
    });

    // The whole point of the typed handle: the callback set is whatever
    // plexus-core says `C`'s members are, resolved by rustc, never a list this
    // crate keeps.
    let callbacks = spec.capability_set.as_ref().map(|c| {
        quote! {
            .with_callbacks(<#c as #core::capability::CapabilitySet>::callbacks())
        }
    });

    let http = http_method_tokens(paths, spec.http_method);
    let auth = auth_tokens(paths, spec.auth);

    let deprecation = spec.deprecation.as_ref().map(|d| {
        let e = deprecation_tokens(paths, d);
        quote! { .with_deprecation(#e) }
    });

    let extensions = spec.extensions.iter().map(|(k, v)| {
        let value = ext_value_tokens(paths, v);
        quote! { .with_extension(#k, #value) }
    });

    quote! {
        #core::ir::MethodIr::new(#name, #dotted)
            #description
            #(#params)*
            #request_context
            #updates
            #terminal
            #callbacks
            .with_http_method(#http)
            .with_auth(#auth)
            #deprecation
            #(#extensions)*
    }
}

fn child_edge_expr(paths: &Paths, edge: &ChildEdgeSpec) -> TokenStream {
    let core = &paths.core;
    let child_ir_fn = syn::Ident::new(CHILD_IR_FN, Span::call_site());

    match edge {
        ChildEdgeSpec::Static {
            namespace,
            child_ty,
        } => {
            // The embedded subtree keeps its own namespace segment from its own
            // document; the parent's edge name is the `#[child]` fn ident, so
            // stamp it to keep `ChildEdge::namespace()` truthful.
            quote! {
                #core::ir::ChildEdge::Static({
                    let mut __child = <#child_ty>::#child_ir_fn()?;
                    __child.namespace = #namespace.to_string();
                    // `ir_hash` is a *document* digest. An embedded subtree is a
                    // node, not a document, so it must not carry one — the
                    // parent's `recompute_hashes` owns the fold from here.
                    __child.ir_hash = ::core::option::Option::None;
                    __child
                })
            }
        }
        ChildEdgeSpec::Dynamic {
            namespace,
            child_ty,
            description,
        } => quote! {
            #core::ir::ChildEdge::Dynamic {
                namespace: #namespace.to_string(),
                // RFC 002 §5.1 — a Dynamic edge MUST carry an advertised hash,
                // "sufficient to fetch and cache it lazily". The *registrations*
                // are a runtime fact; the *shape* they all have is not — the
                // signature names it. So the backend advertises the declared
                // child type's own node hash, and a changed one is exactly the
                // refetch signal. The subtree itself is deliberately still not
                // embedded (§5.1), and no consumer recomputes this (§5.2).
                //
                // Unlike a Static edge the child's namespace is NOT restamped:
                // there is no embedded node here to rename, and the hash a
                // registration reports when fetched is the child type's own.
                hash: <#child_ty>::#child_ir_fn()?.hash,
                description: #description.to_string(),
            }
        },
        ChildEdgeSpec::Indexed {
            namespace,
            list_method,
            search_method,
            id_field,
            path_template,
            child_ty,
            description,
        } => {
            let search = match search_method {
                Some(s) => quote! { ::core::option::Option::Some(#s.to_string()) },
                None => quote! { ::core::option::Option::None },
            };
            quote! {
                #core::ir::ChildEdge::Indexed {
                    namespace: #namespace.to_string(),
                    list_method: #list_method.to_string(),
                    search_method: #search,
                    id_field: #id_field.to_string(),
                    path_template: #path_template.to_string(),
                    template: ::std::boxed::Box::new({
                        let mut __child = <#child_ty>::#child_ir_fn()?;
                        __child.namespace = #namespace.to_string();
                        __child.ir_hash = ::core::option::Option::None;
                        __child
                    }),
                    description: #description.to_string(),
                }
            }
        }
    }
}

/// The `ActivationIr` statement sequence, ending in `Ok(ir)`. Valid only inside
/// a `Result`-returning scope.
pub(crate) fn activation_expr(paths: &Paths, spec: &ActivationIrSpec) -> TokenStream {
    let core = &paths.core;
    let namespace = &spec.namespace;
    let version = &spec.version;

    let backend_name = spec.backend_name.as_ref().map(|b| {
        quote! { .with_backend_name(#b) }
    });
    let respond_method = spec.respond_method.as_ref().map(|r| {
        quote! { .with_respond_method(#r) }
    });
    let description = (!spec.description.is_empty()).then(|| {
        let d = &spec.description;
        quote! { .with_description(#d) }
    });
    let long_description = spec.long_description.as_ref().filter(|d| !d.is_empty()).map(|d| {
        quote! { .with_long_description(#d) }
    });
    let deprecation = spec.deprecation.as_ref().map(|d| {
        let e = deprecation_tokens(paths, d);
        quote! { .with_deprecation(#e) }
    });
    // RFC 002 §6.9.1. Declared on the activation that owns the methods; §6.9.3
    // forbids ancestor resolution, so nothing here is pushed into a child's
    // expression — an embedded child's own `activation_ir!` declares its own or
    // has none.
    let request_context = spec.request_context.as_ref().map(|t| {
        let s = schema_ref(paths, t);
        quote! { .with_request_context(#s) }
    });

    let methods = spec.methods.iter().map(|m| {
        let e = method_expr(paths, m);
        quote! { .with_method(#e) }
    });
    let children = spec.children.iter().map(|c| {
        let e = child_edge_expr(paths, c);
        quote! { .with_child(#e) }
    });

    quote! {
        let mut __ir = #core::ir::ActivationIr::new(#namespace, #version)
            #backend_name
            #respond_method
            #description
            #long_description
            #request_context
            #deprecation
            #(#methods)*
            #(#children)*;
        __ir.recompute_hashes();
        ::core::result::Result::Ok(__ir)
    }
}
