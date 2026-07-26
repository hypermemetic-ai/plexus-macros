//! PLX-91 (M1·F) — the handler table.
//!
//! This module is the whole of what the switchover replaced ~2,260 lines of
//! `quote!` templates with. Where `codegen/activation.rs` used to emit, per
//! method arm, a re-monomorphized copy of *five* concerns — method lookup, auth
//! injection, request extraction, param decode, and wrap/terminal plumbing —
//! it now emits **one** entry in a
//! `Vec<(&'static str, ErasedHandler<Self>)>`, and the closure inside it does
//! the only two things that genuinely need the type:
//!
//! 1. typed param decode, and
//! 2. the call to the user's own method.
//!
//! Everything else is `plexus_core::runtime::entry_with_state`, as ordinary
//! Rust: method lookup is a hash probe against the [`ActivationIr`], auth
//! injection is `AuthRequirementIr` read off the method IR before anything
//! runs, request extraction is `TurnRequest`, and the terminal is guaranteed
//! exactly-once by the turn loop rather than by a `wrap_stream` template.
//!
//! # State is a parameter (PLX-97)
//!
//! Every closure emitted here **captures nothing**. It receives `Arc<Self>` as
//! its first argument, which is what makes the table constructible from inside
//! a `&self` method and `'static` regardless of where it was built. That was
//! the seam PLX-91's first attempt stopped on and PLX-97 closed; the owned
//! handle arrives through `Activation::call_arc(self: Arc<Self>, …)`.
//!
//! # The one construct that did not move: `bidirectional`
//!
//! A `#[method]` taking a `&Arc<BidirChannel<Req, Resp>>` keeps the legacy
//! dispatch arm, emitted by [`super::activation::generate_bidir_arms`]. This is
//! deliberate and it is *not* a silent semantic change — a bidirectional method
//! dispatches today exactly as it dispatched before the switchover. The reason
//! it cannot move is structural rather than incidental: the turn model's
//! server-to-client channel is `TurnContext::client::<C>()`, whose capability
//! set `C` is a **type** the method must declare; a `BidirChannel<Req, Resp>`
//! declares a request/response *pair*, and `MethodIr::callbacks` has no slot
//! for one (the same gap PLX-91's requirement 4 records for
//! `MethodSchema::{bidirectional, request_type, response_type}`). Moving it
//! would mean inventing a capability for every bidir pair, i.e. extending the
//! frozen IR — the trade decision gate 1 forbids. ACP depends on bidirectional,
//! so it is kept working rather than degraded.

use proc_macro2::TokenStream;
use quote::quote;

use crate::parse::{BidirType, MethodInfo};

/// Rewrite `self` to `__me` inside a `#[from_auth(...)]` resolver expression.
///
/// `#[from_auth(self.validate_user)]` names a method on the activation. On the
/// deleted dispatch arms that expression was expanded inside
/// `Activation::call(&self, ..)`, where `self` was in scope. A handler closure
/// captures **nothing** — that is what makes it `'static` (PLX-97) — so the
/// activation arrives as the closure's first argument, `__me: Arc<Self>`, and
/// the resolver has to say so. The rewrite is token-level and total: every
/// `self` ident in the expression becomes `__me`, which is exactly the
/// substitution the move from a method body to a closure body performs.
fn rebind_self(tokens: TokenStream) -> TokenStream {
    use proc_macro2::{Group, TokenTree};
    tokens
        .into_iter()
        .map(|tt| match tt {
            TokenTree::Ident(ref i) if i == "self" => {
                TokenTree::Ident(proc_macro2::Ident::new("__me", i.span()))
            }
            TokenTree::Group(g) => {
                TokenTree::Group(Group::new(g.delimiter(), rebind_self(g.stream())))
            }
            other => other,
        })
        .collect()
}

/// Is this method one the switchover can serve from the handler table?
pub(crate) fn is_switched(m: &MethodInfo) -> bool {
    matches!(m.bidirectional, BidirType::None)
}

/// `Vec<(&'static str, ErasedHandler<Self>)>` — the whole of dispatch.
pub(crate) fn generate_handler_table(
    methods: &[MethodInfo],
    crate_path: &syn::Path,
    request_type: Option<&syn::Type>,
    deps: &syn::Ident,
) -> TokenStream {
    let entries: Vec<TokenStream> = methods
        .iter()
        .filter(|m| is_switched(m))
        .map(|m| handler_entry(m, crate_path, request_type, deps))
        .collect();

    quote! {
        /// PLX-91: this activation's handler table — one erased closure per
        /// `#[method]`, each capturing nothing and receiving `Arc<Self>`.
        ///
        /// Built per call rather than cached in a `static`: `HandlerTable<Self>`
        /// names `Self`, and a `static` inside a generic impl cannot. The cost
        /// is one `Arc` per method per turn, against a dispatch path that is
        /// already double-erased.
        #[doc(hidden)]
        #[allow(dead_code, unused_variables, unused_mut, clippy::redundant_closure)]
        pub fn __plexus_handler_table() -> #crate_path::runtime::HandlerTable<Self> {
            #crate_path::runtime::HandlerTable::new(::std::vec![
                #(#entries,)*
            ])
        }
    }
}

fn handler_entry(
    m: &MethodInfo,
    crate_path: &syn::Path,
    request_type: Option<&syn::Type>,
    deps: &syn::Ident,
) -> TokenStream {
    let method_name = &m.method_name;
    let fn_name = &m.fn_name;

    let auth_injection = auth_injection(m, crate_path);
    let activation_extraction = activation_extraction(m, crate_path, request_type);
    let activation_param_bindings = activation_param_bindings(m, request_type);
    let resolver_bindings: Vec<TokenStream> = m
        .auth_resolvers
        .iter()
        .map(|r| {
            let name = &r.param_name;
            let raw = &r.resolver_expr;
            let expr = rebind_self(quote! { #raw });
            quote! {
                let #name = #expr(&__auth_ctx).await.map_err(|e| {
                    #crate_path::runtime::TurnError::new(
                        "plexus.execution_error",
                        e.to_string(),
                    )
                })?;
            }
        })
        .collect();

    let (param_extraction, param_names) = param_extraction(m, crate_path, deps);
    let call_args = call_args(m, &param_names);

    quote! {
        (
            #method_name,
            #crate_path::runtime::ErasedHandler::<Self>::stateful(
                |__me, __input| async move {
                    let __turn = __input.turn;
                    let __params = __input.params;
                    #auth_injection
                    #activation_extraction
                    #(#activation_param_bindings)*
                    #(#resolver_bindings)*
                    #param_extraction

                    let __stream = __me.#fn_name(#call_args).await;

                    // The update/terminal split, made by the runtime rather
                    // than by a `wrap_stream` template: every stream item is
                    // an update, and the turn's single terminal is produced by
                    // `entry`'s loop once this future resolves.
                    {
                        use #deps::futures::StreamExt;
                        let mut __stream = ::std::boxed::Box::pin(__stream);
                        while let Some(__item) = __stream.next().await {
                            __turn.emit_typed(&__item).await?;
                        }
                    }
                    ::core::result::Result::Ok(
                        #crate_path::runtime::TurnOutcome::complete()
                    )
                }
            ),
        )
    }
}

/// Auth injection, read off the turn rather than off a per-arm template.
///
/// `entry` has already enforced `AuthRequirementIr::Required` before the
/// handler is reachable, so the `ok_or_else` below is a belt-and-braces
/// re-check rather than the gate — the gate moved into the runtime, which is
/// exactly requirement 2.
fn auth_injection(m: &MethodInfo, crate_path: &syn::Path) -> TokenStream {
    if !m.requires_auth {
        return quote! {};
    }
    if m.auth_optional && m.auth_resolvers.is_empty() {
        quote! {
            let __auth_ctx: ::core::option::Option<&#crate_path::plexus::AuthContext> =
                __turn.auth();
        }
    } else {
        quote! {
            let __auth_ctx = __turn.auth().ok_or_else(|| {
                #crate_path::runtime::TurnError::new(
                    "plexus.unauthenticated",
                    "This method requires authentication",
                )
            })?;
        }
    }
}

fn activation_extraction(
    m: &MethodInfo,
    crate_path: &syn::Path,
    request_type: Option<&syn::Type>,
) -> TokenStream {
    let skip = matches!(
        &m.request_override,
        Some(crate::parse::MethodRequestOverride::Skip)
    );
    if skip {
        return quote! {};
    }
    let Some(req_ty) = request_type else {
        return quote! {};
    };

    if !m.activation_params.is_empty() {
        quote! {
            let __raw_ctx = __turn.raw_context().ok_or_else(|| {
                #crate_path::runtime::TurnError::new(
                    "plexus.unauthenticated",
                    "no request context",
                )
            })?;
            let __req = <#req_ty as #crate_path::request::PlexusRequest>::extract(__raw_ctx)
                .map_err(|e| {
                    #crate_path::runtime::TurnError::new(
                        "plexus.execution_error",
                        e.to_string(),
                    )
                })?;
        }
    } else {
        quote! {
            if let Some(__raw_ctx) = __turn.raw_context() {
                <#req_ty as #crate_path::request::PlexusRequest>::extract(__raw_ctx)
                    .map_err(|e| {
                        #crate_path::runtime::TurnError::new(
                            "plexus.invalid_params",
                            e.to_string(),
                        )
                    })?;
            }
        }
    }
}

fn activation_param_bindings(m: &MethodInfo, request_type: Option<&syn::Type>) -> Vec<TokenStream> {
    let skip = matches!(
        &m.request_override,
        Some(crate::parse::MethodRequestOverride::Skip)
    );
    if m.activation_params.is_empty() || skip || request_type.is_none() {
        return Vec::new();
    }
    m.activation_params
        .iter()
        .map(|ap| {
            let name = &ap.param_name;
            let ty = &ap.ty;
            quote! { let #name: #ty = __req.#name.clone(); }
        })
        .collect()
}

/// The argument list handed to the user's method, in the order its signature
/// declares: activation params, then `#[from_auth]` bindings, then the wire
/// parameters — with the raw auth context in front when the method takes one
/// directly. Identical to the ordering the deleted dispatch arms used.
fn call_args(m: &MethodInfo, param_names: &[&syn::Ident]) -> TokenStream {
    let activation_param_names: Vec<&syn::Ident> =
        m.activation_params.iter().map(|ap| &ap.param_name).collect();
    let resolver_names: Vec<&syn::Ident> =
        m.auth_resolvers.iter().map(|r| &r.param_name).collect();

    if m.requires_auth && m.auth_resolvers.is_empty() {
        quote! { __auth_ctx, #(#param_names),* }
    } else if !m.activation_params.is_empty() && !m.auth_resolvers.is_empty() {
        quote! { #(#activation_param_names,)* #(#resolver_names,)* #(#param_names),* }
    } else if !m.activation_params.is_empty() {
        quote! { #(#activation_param_names,)* #(#param_names),* }
    } else if !m.auth_resolvers.is_empty() {
        quote! { #(#resolver_names,)* #(#param_names),* }
    } else {
        quote! { #(#param_names),* }
    }
}

/// Typed param decode — the one concern that genuinely needs monomorphization,
/// which is why it is the one concern still emitted per method.
///
/// The decoding *rules* are byte-for-byte the ones the deleted dispatch arms
/// used (object-or-bare-value, `Option<T>` treats an explicit `null` as
/// missing, a missing required field names itself); only the error type
/// changed, from a pre-flight `PlexusError::InvalidParams` to a `TurnError`
/// that resolves the turn as `Failed`. That is the update/terminal split doing
/// its job: a decode failure is now a structured terminal on a real turn
/// rather than an error with no turn behind it.
fn param_extraction<'a>(
    m: &'a MethodInfo,
    crate_path: &syn::Path,
    deps: &syn::Ident,
) -> (TokenStream, Vec<&'a syn::Ident>) {
    let param_names: Vec<&syn::Ident> = m.params.iter().map(|p| &p.name).collect();

    let extraction = match m.params.len() {
        0 => quote! {},
        1 => {
            let param = &m.params[0];
            let param_name = &param.name;
            let param_str = param_name.to_string();
            let is_option = crate::codegen::method_enum::is_option_type(&param.ty);

            if is_option {
                quote! {
                    let #param_name = match &__params {
                        #deps::serde_json::Value::Object(map) => {
                            map.get(#param_str)
                                .filter(|v| !v.is_null())
                                .map(|v| #deps::serde_json::from_value(v.clone()))
                                .transpose()
                                .map_err(|e| #crate_path::runtime::TurnError::new(
                                    "plexus.invalid_params", e.to_string()))?
                        }
                        #deps::serde_json::Value::Null => None,
                        _ => #deps::serde_json::from_value(__params.clone())
                            .map_err(|e| #crate_path::runtime::TurnError::new(
                                "plexus.invalid_params", e.to_string()))?,
                    };
                }
            } else {
                quote! {
                    let #param_name = match &__params {
                        #deps::serde_json::Value::Object(map) => {
                            if let Some(val) = map.get(#param_str) {
                                #deps::serde_json::from_value(val.clone())
                                    .map_err(|e| #crate_path::runtime::TurnError::new(
                                        "plexus.invalid_params", e.to_string()))?
                            } else {
                                return ::core::result::Result::Err(
                                    #crate_path::runtime::TurnError::new(
                                        "plexus.invalid_params",
                                        format!("missing field: {}", #param_str),
                                    )
                                );
                            }
                        }
                        _ => #deps::serde_json::from_value(__params.clone())
                            .map_err(|e| #crate_path::runtime::TurnError::new(
                                "plexus.invalid_params", e.to_string()))?,
                    };
                }
            }
        }
        _ => {
            let extractions: Vec<TokenStream> = m
                .params
                .iter()
                .map(|p| {
                    let name = &p.name;
                    let name_str = name.to_string();
                    let is_option = crate::codegen::method_enum::is_option_type(&p.ty);

                    if is_option {
                        quote! {
                            let #name = __map.get(#name_str)
                                .filter(|v| !v.is_null())
                                .map(|v| #deps::serde_json::from_value(v.clone()))
                                .transpose()
                                .map_err(|e| #crate_path::runtime::TurnError::new(
                                    "plexus.invalid_params", e.to_string()))?;
                        }
                    } else {
                        quote! {
                            let #name = __map.get(#name_str)
                                .ok_or_else(|| #crate_path::runtime::TurnError::new(
                                    "plexus.invalid_params",
                                    format!("missing field: {}", #name_str)))
                                .and_then(|v| #deps::serde_json::from_value(v.clone())
                                    .map_err(|e| #crate_path::runtime::TurnError::new(
                                        "plexus.invalid_params", e.to_string())))?;
                        }
                    }
                })
                .collect();

            quote! {
                let __map = __params.as_object()
                    .ok_or_else(|| #crate_path::runtime::TurnError::new(
                        "plexus.invalid_params", "expected object"))?;
                #(#extractions)*
            }
        }
    };

    (extraction, param_names)
}
