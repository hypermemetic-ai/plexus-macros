//! PLX-136 (ACP·B) — `#[acp_agent]`: advertising and implementing are one act.
//!
//! ACP's rule is that an omitted capability MUST be treated as unsupported.
//! The failure that rule invites is a *second* place where capability truth
//! lives: a hand-written `AgentCapabilities` saying `loadSession: true` next
//! to an agent that never implemented `session/load`, or the reverse — an
//! implementation nobody is ever told about.
//!
//! `#[acp_agent]` closes both directions by construction:
//!
//! - The author writes an **inherent** `impl` block whose items are ACP v1
//!   methods. The macro moves those bodies aside and emits
//!   `impl plexus_acp::v1::Agent for T` in their place, so a misspelled item
//!   is `E0407` (with the expected signature printed) and a missing required
//!   item is `E0046` (with the whole surface enumerated).
//! - The macro **generates `initialize` itself**, and writing `initialize` in
//!   an `#[acp_agent]` block is a compile error. `AgentCapabilities` is
//!   therefore not something an author can spell — there is no attribute key
//!   for it and no method body that could return one.
//! - The capabilities come from [`plexus_acp::v1::derive_agent_capabilities`],
//!   fed the idents the author actually wrote.
//!
//! # This macro owns no ACP vocabulary
//!
//! A second vocabulary model is the failure this project has designed away
//! three times. So the macro emits the author's own idents as strings and
//! lets `plexus-acp` do the mapping at runtime, through its public
//! `AgentMethod::ALL` / `AgentMethod::trait_item()` surface. `plexus-macros`
//! does not depend on `plexus-acp`, spells no wire name, and spells no
//! capability field.
//!
//! It does name **three trait-item idents** — `initialize` (which it
//! generates), `new_session` (which it wraps for the per-session obligation)
//! and `authenticate` (whose presence decides whether the generated
//! `initialize` reaches for auth methods). Those are structural, not
//! vocabulary, and `tests/plx136_acp_agent_tests.rs` pins them against
//! `AgentMethod::trait_item()` so a rename upstream fails a test instead of
//! silently disarming an obligation. The generated code additionally carries
//! a runtime drift check against `Derived::requires_auth_methods`.
//!
//! # The three advertisement sites
//!
//! PLX-135 handed this node three, not one:
//!
//! 1. **`AgentCapabilities`**, returned once from `initialize` — seven of the
//!    nine optional items. Written only by the derivation.
//! 2. **`NewSessionResponse.modes` / `.configOptions`**, per session — where
//!    `session/set_mode` and `session/set_config_option` are advertised.
//!    Checked in the generated `new_session` wrapper, in both directions,
//!    driven by `Derived::per_session` and the `Advertisement::PerSession`
//!    payload rather than by anything this crate knows.
//! 3. **`InitializeResponse.authMethods`**, gating `authenticate`. The
//!    generated `initialize` calls `Self::acp_auth_methods()` if and only if
//!    `authenticate` is implemented, so auth methods cannot be advertised
//!    without the impl, and an empty list with the impl is an error.

use proc_macro2::{Span, TokenStream};
use proc_macro_crate::{crate_name, FoundCrate};
use quote::{format_ident, quote};
use syn::{
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
    spanned::Spanned,
    Expr, ExprLit, FnArg, ImplItem, ImplItemFn, ItemImpl, Lit, Meta, Pat, ReturnType, Token,
};

/// The item the macro generates rather than accepts.
const GENERATED_ITEM: &str = "initialize";
/// The item the macro wraps to discharge advertisement site 2.
const WRAPPED_ITEM: &str = "new_session";
/// The item whose presence turns on advertisement site 3.
const AUTH_ITEM: &str = "authenticate";

// ---------------------------------------------------------------------------
// attribute keys
// ---------------------------------------------------------------------------

/// The complete `#[acp_agent(...)]` key set.
///
/// **This is the c2 surface.** Nothing here declares a capability, and
/// nothing can: there is no `capabilities`, no `load_session`, no
/// `auth_methods`, no `ext`. PLX-93/PLX-147 made unknown keys an error on the
/// `#[method]` and `#[activation]` paths; this is a *third* allowlist for a
/// *third* parser, deliberately separate so that adding a key here cannot
/// loosen `#[activation]` (and vice versa).
///
/// `name` and `version` populate `InitializeResponse.agentInfo`, which is
/// identity, not capability — an agent that says who it is advertises
/// nothing about what it can do.
///
/// `acp_path` is the same escape hatch `crate_path` is on `#[activation]`:
/// it renames the crate the generated code calls, so a consumer who points
/// it at a crate other than `plexus-acp` gets whatever derivation that crate
/// implements. That is vocabulary substitution, not capability declaration —
/// there is still no way to say `loadSession: true` — but it is the one
/// input that can change what the derivation returns, and it is named here
/// rather than left implicit.
pub(crate) const ACP_AGENT_KEYS: &[&str] = &["acp_path", "name", "version"];

/// Keys someone would plausibly reach for to declare a capability by hand,
/// each answered with the impl to write instead.
///
/// This is the compile-fail half of c2: the second source of truth does not
/// exist, and every spelling of it that an author might try says so by name.
fn acp_agent_key_hint(key: &str) -> Option<&'static str> {
    Some(match key {
        "capabilities" | "agent_capabilities" | "agentCapabilities" => {
            "`AgentCapabilities` is DERIVED from the items in this impl block \
             and cannot be declared. To advertise `loadSession`, implement \
             `load_session`; to advertise `sessionCapabilities.list`, \
             implement `list_sessions`. There is no second place where \
             capability truth lives"
        }
        "load_session"
        | "loadSession"
        | "session_capabilities"
        | "sessionCapabilities"
        | "logout"
        | "auth" => {
            "capabilities are not declared — implement the method. The optional \
             `plexus_acp::v1::Agent` items are `authenticate`, `logout`, \
             `load_session`, `resume_session`, `list_sessions`, \
             `delete_session`, `close_session`, `set_session_mode` and \
             `set_session_config_option`, and implementing one is the only way \
             to advertise it"
        }
        "auth_methods" | "authMethods" => {
            "`InitializeResponse.authMethods` is not declared here. Implement \
             `authenticate` and provide `fn acp_auth_methods() -> Vec<AuthMethod>` \
             in a separate inherent impl; the generated `initialize` calls it \
             only when `authenticate` is implemented, so auth methods cannot be \
             advertised without the method that services them"
        }
        "modes" | "config_options" | "configOptions" => {
            "session modes and config options are advertised PER SESSION, on \
             the `NewSessionResponse` your `new_session` returns — not at \
             initialize time. Implement `set_session_mode` / \
             `set_session_config_option` and populate the field there"
        }
        "ext" => {
            "`ext(...)` is not a mapping mechanism for ACP. The vocabulary is \
             closed and the trait is the mapping; ACP's own extension channel \
             is the reserved underscore-prefixed method namespace and the \
             `_meta` fields on the wire types, neither of which is declared here"
        }
        "namespace" | "crate_path" | "description" | "request" | "version_" => {
            "that key belongs to `#[plexus_macros::activation]`, which is a \
             different macro with a different allowlist"
        }
        "protocol_version" | "protocolVersion" => {
            "the protocol version is not declared — `plexus_acp::v1::Agent` IS \
             version 1, and the generated `initialize` answers \
             `ProtocolVersion::V1`. A v2 agent will be `plexus_acp::v2::Agent`"
        }
        _ => return None,
    })
}

fn unknown_acp_agent_key(span: Span, key: &str) -> syn::Error {
    let message = match acp_agent_key_hint(key) {
        Some(hint) => format!("unknown `#[acp_agent(...)]` key `{key}`: {hint}"),
        None => format!(
            "unknown `#[acp_agent(...)]` key `{key}`. Accepted: {}. That is the \
             whole surface — capabilities are derived from the items in the \
             impl block, never declared",
            ACP_AGENT_KEYS.join(", "),
        ),
    };
    syn::Error::new(span, message)
}

/// Parsed `#[acp_agent(...)]` arguments.
pub(crate) struct AcpAgentAttrs {
    /// Override for the path to `plexus_acp` in generated code.
    acp_path: Option<syn::Path>,
    /// `agentInfo.name`.
    name: Option<syn::LitStr>,
    /// `agentInfo.version`.
    version: Option<syn::LitStr>,
    span: Span,
}

impl Parse for AcpAgentAttrs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let span = input.span();
        let mut attrs = AcpAgentAttrs {
            acp_path: None,
            name: None,
            version: None,
            span,
        };

        if input.is_empty() {
            return Ok(attrs);
        }

        let metas = Punctuated::<Meta, Token![,]>::parse_terminated(input)?;
        for meta in metas {
            let key_span = meta.path().span();
            let key = meta
                .path()
                .get_ident()
                .map(ToString::to_string)
                .unwrap_or_else(|| quote::ToTokens::to_token_stream(meta.path()).to_string());

            // The gate runs BEFORE any arm, so no arm can swallow a typo.
            if !ACP_AGENT_KEYS.contains(&key.as_str()) {
                return Err(unknown_acp_agent_key(key_span, &key));
            }

            let Meta::NameValue(nv) = meta else {
                return Err(syn::Error::new(
                    key_span,
                    format!("`#[acp_agent(...)]` key `{key}` must be written as `{key} = \"...\"`"),
                ));
            };
            let Expr::Lit(ExprLit {
                lit: Lit::Str(lit), ..
            }) = nv.value
            else {
                return Err(syn::Error::new(
                    key_span,
                    format!("`#[acp_agent(...)]` key `{key}` must be written as `{key} = \"...\"`"),
                ));
            };

            match key.as_str() {
                "acp_path" => attrs.acp_path = Some(lit.parse()?),
                "name" => attrs.name = Some(lit),
                "version" => attrs.version = Some(lit),
                _ => unreachable!("gated by ACP_AGENT_KEYS"),
            }
        }

        // `Implementation::new` takes both; one without the other would make
        // agentInfo half-true, so it is an error rather than a default.
        match (&attrs.name, &attrs.version) {
            (Some(_), None) => {
                return Err(syn::Error::new(
                    attrs.name.as_ref().unwrap().span(),
                    "`name` needs `version` — `InitializeResponse.agentInfo` carries both, \
                     and half of it is not a thing the wire can say",
                ))
            }
            (None, Some(_)) => {
                return Err(syn::Error::new(
                    attrs.version.as_ref().unwrap().span(),
                    "`version` needs `name` — `InitializeResponse.agentInfo` carries both, \
                     and half of it is not a thing the wire can say",
                ))
            }
            _ => {}
        }

        Ok(attrs)
    }
}

// ---------------------------------------------------------------------------
// codegen
// ---------------------------------------------------------------------------

/// Locate `plexus-acp` in the consumer's manifest, or take the override.
fn resolve_acp_path(explicit: Option<&syn::Path>, err_span: Span) -> syn::Result<syn::Path> {
    if let Some(p) = explicit {
        return Ok(p.clone());
    }
    match crate_name("plexus-acp") {
        Ok(FoundCrate::Itself) => syn::parse_str("crate"),
        Ok(FoundCrate::Name(name)) => {
            let ident = syn::Ident::new(&name, err_span);
            Ok(syn::parse_quote!(::#ident))
        }
        Err(e) => Err(syn::Error::new(
            err_span,
            format!(
                "#[plexus_macros::acp_agent] could not locate the `plexus-acp` dependency \
                 in Cargo.toml ({e}). Add `plexus-acp`, or pass `acp_path = \"...\"` to \
                 override."
            ),
        )),
    }
}

/// One author-written ACP method, decomposed.
struct AuthorMethod {
    ident: syn::Ident,
    mangled: syn::Ident,
    /// Args after the receiver, in order.
    args: Vec<syn::PatType>,
    arg_names: Vec<syn::Ident>,
    /// The author's return type, copied verbatim so a wrong one is `E0053`
    /// pointing at the author's tokens.
    output: syn::Type,
}

fn decompose(f: &ImplItemFn) -> syn::Result<AuthorMethod> {
    let span = f.sig.ident.span();

    if f.sig.asyncness.is_none() {
        return Err(syn::Error::new(
            span,
            "`#[acp_agent]` items are `async fn` — every `plexus_acp::v1::Agent` \
             method is async",
        ));
    }
    if !f.sig.generics.params.is_empty() {
        return Err(syn::Error::new(
            f.sig.generics.span(),
            "`#[acp_agent]` items take no generic parameters: the ACP vocabulary is \
             closed, so every signature is fixed by the trait",
        ));
    }

    let mut inputs = f.sig.inputs.iter();
    match inputs.next() {
        Some(FnArg::Receiver(r)) if r.reference.is_some() && r.mutability.is_none() => {}
        Some(other) => {
            return Err(syn::Error::new(
                other.span(),
                "`#[acp_agent]` items take `&self`: `plexus_acp::v1::Agent` is \
                 object-safe and shared across concurrent turns",
            ))
        }
        None => {
            return Err(syn::Error::new(
                span,
                "`#[acp_agent]` items take `&self` as their first parameter",
            ))
        }
    }

    let mut args = Vec::new();
    let mut arg_names = Vec::new();
    for input in inputs {
        let FnArg::Typed(pt) = input else {
            return Err(syn::Error::new(input.span(), "unexpected receiver"));
        };
        let Pat::Ident(pi) = &*pt.pat else {
            return Err(syn::Error::new(
                pt.pat.span(),
                "`#[acp_agent]` items bind their parameters to plain identifiers",
            ));
        };
        args.push(pt.clone());
        arg_names.push(pi.ident.clone());
    }

    let output = match &f.sig.output {
        ReturnType::Type(_, ty) => (**ty).clone(),
        ReturnType::Default => {
            return Err(syn::Error::new(
                span,
                "`#[acp_agent]` items return `plexus_acp::v1::Result<..>` — every ACP \
                 method answers, and a bare `()` cannot carry the protocol's error codes",
            ))
        }
    };

    Ok(AuthorMethod {
        mangled: format_ident!("__acp_author_{}", f.sig.ident),
        ident: f.sig.ident.clone(),
        args,
        arg_names,
        output,
    })
}

/// The desugared `#[async_trait]` receiver shape, written out so that
/// generated code needs no `async-trait` dependency in the consumer.
fn boxed_future(output: &syn::Type) -> TokenStream {
    quote! {
        ::core::pin::Pin<::std::boxed::Box<
            dyn ::core::future::Future<Output = #output>
                + ::core::marker::Send
                + '__acp_async_trait
        >>
    }
}

/// Expand `#[acp_agent]`.
pub(crate) fn generate(attrs: &AcpAgentAttrs, input: ItemImpl) -> syn::Result<TokenStream> {
    if let Some((_, path, _)) = &input.trait_ {
        return Err(syn::Error::new(
            path.span(),
            "`#[acp_agent]` goes on an INHERENT impl block — it generates the \
             `plexus_acp::v1::Agent` impl for you, which is what makes a \
             misspelled item `E0407` and a missing required item `E0046`",
        ));
    }

    let acp = resolve_acp_path(attrs.acp_path.as_ref(), attrs.span)?;
    let self_ty = &input.self_ty;
    let (impl_generics, _, where_clause) = input.generics.split_for_impl();

    // ---- read the author's block ----
    let mut methods: Vec<AuthorMethod> = Vec::new();
    let mut author_items: Vec<TokenStream> = Vec::new();
    for item in &input.items {
        let ImplItem::Fn(f) = item else {
            return Err(syn::Error::new(
                item.span(),
                "`#[acp_agent]` blocks contain only ACP method implementations. \
                 Constants, types and helper functions go in a separate `impl` block \
                 — every item here is moved into the generated \
                 `plexus_acp::v1::Agent` impl, where a non-ACP item is `E0407`",
            ));
        };

        if f.sig.ident == GENERATED_ITEM {
            return Err(syn::Error::new(
                f.sig.ident.span(),
                "`#[acp_agent]` GENERATES `initialize` — it is not yours to write. \
                 An author-written `initialize` would be the second place where \
                 capability truth lives: it returns `AgentCapabilities`, and a \
                 hand-written one can say `loadSession: true` with no `load_session` \
                 impl, or omit a capability for a method that exists. The generated \
                 one derives capabilities from the items in this block and there is \
                 no way to override it. Use `name`/`version` on the attribute for \
                 `agentInfo`",
            ));
        }

        let decomposed = decompose(f)?;

        // The author's body, kept under a mangled name so the trait wrapper can
        // own the real one. Attributes and doc comments ride along.
        let attrs_kept = &f.attrs;
        let vis = &f.vis;
        let mangled = &decomposed.mangled;
        let inputs = &f.sig.inputs;
        let out = &decomposed.output;
        let body = &f.block;
        author_items.push(quote! {
            #(#attrs_kept)*
            #[doc(hidden)]
            #[allow(clippy::used_underscore_items)]
            #vis async fn #mangled(#inputs) -> #out #body
        });

        methods.push(decomposed);
    }

    let implemented_idents: Vec<String> = methods.iter().map(|m| m.ident.to_string()).collect();
    let authenticate_implemented = implemented_idents.iter().any(|i| i == AUTH_ITEM);

    // ---- the derivation ----
    let idents_lit = implemented_idents.iter().map(|s| s.as_str());
    let derived_impl = quote! {
        #[automatically_derived]
        impl #impl_generics #self_ty #where_clause {
            /// Every `plexus_acp::v1::Agent` item this type implements, as the
            /// idents `#[acp_agent]` read off the impl block.
            ///
            /// This is the *only* input to capability derivation.
            pub const ACP_IMPLEMENTED_ITEMS: &'static [&'static str] = &[#(#idents_lit),*];

            /// `AgentCapabilities` plus the two obligations that are not
            /// capability fields, derived from
            /// [`Self::ACP_IMPLEMENTED_ITEMS`].
            ///
            /// The mapping from ident to `AgentMethod` is done by `plexus-acp`
            /// itself, through its public `AgentMethod::ALL` /
            /// `AgentMethod::trait_item()` surface — `plexus-macros` holds no
            /// copy of the ACP vocabulary.
            #[must_use]
            pub fn acp_derived() -> #acp::v1::Derived {
                #acp::v1::derive_agent_capabilities(
                    #acp::v1::AgentMethod::ALL.into_iter().filter(|__m| {
                        Self::ACP_IMPLEMENTED_ITEMS
                            .contains(&#acp::v1::AgentMethod::trait_item(*__m))
                    }),
                )
            }
        }
    };

    // ---- generated `initialize` (advertisement sites 1 and 3) ----
    let agent_info = match (&attrs.name, &attrs.version) {
        (Some(n), Some(v)) => quote! {
            __response.agent_info = ::core::option::Option::Some(
                #acp::v1::schema::Implementation::new(#n, #v)
            );
        },
        _ => quote! {},
    };

    let auth_methods_block = if authenticate_implemented {
        quote! {
            // Site 3. Reached ONLY because `authenticate` is implemented, so
            // there is no input that advertises auth methods without it.
            let __methods: ::std::vec::Vec<#acp::v1::schema::AuthMethod> =
                Self::acp_auth_methods();
            if __methods.is_empty() {
                return ::core::result::Result::Err(__acp_err(
                    "this agent implements `authenticate`, so \
                     `InitializeResponse.authMethods` MUST be non-empty, but \
                     `acp_auth_methods()` returned none: an implemented method \
                     would never be advertised",
                ));
            }
            __response.auth_methods = __methods;
        }
    } else {
        quote! {}
    };

    let authenticate_flag = authenticate_implemented;
    let init_output: syn::Type =
        syn::parse_quote!(#acp::v1::Result<#acp::v1::schema::InitializeResponse>);
    let init_ret = boxed_future(&init_output);

    let initialize_fn = quote! {
        fn initialize<'__acp_life0, '__acp_async_trait>(
            &'__acp_life0 self,
            request: #acp::v1::schema::InitializeRequest,
        ) -> #init_ret
        where
            '__acp_life0: '__acp_async_trait,
            Self: '__acp_async_trait,
        {
            ::std::boxed::Box::pin(async move {
                fn __acp_err(message: &str) -> #acp::v1::schema::Error {
                    let mut __e = #acp::v1::schema::Error::internal_error();
                    __e.message = ::std::string::ToString::to_string(message);
                    __e
                }

                let _ = &request;
                let __derived = Self::acp_derived();

                // Drift guard: `plexus-acp` decides which item gates
                // `authMethods`; `#[acp_agent]` had to name one ident at macro
                // time to decide whether to emit the call below. If the two ever
                // disagree, fail closed rather than advertise nothing.
                if __derived.requires_auth_methods != #authenticate_flag {
                    return ::core::result::Result::Err(__acp_err(
                        "plexus-acp and #[acp_agent] disagree about which item gates \
                         `InitializeResponse.authMethods`; refusing to initialize \
                         rather than answer with an advertisement that is not true",
                    ));
                }

                let mut __response = #acp::v1::schema::InitializeResponse::new(
                    #acp::v1::ProtocolVersion::V1,
                );
                // Site 1. The ONLY write to `agent_capabilities` anywhere.
                __response.agent_capabilities = __derived.capabilities;
                #agent_info
                #auth_methods_block

                ::core::result::Result::Ok(__response)
            })
        }
    };

    // ---- wrappers ----
    let mut wrappers = Vec::new();
    for m in &methods {
        let ident = &m.ident;
        let mangled = &m.mangled;
        let args = &m.args;
        let names = &m.arg_names;
        let out = &m.output;
        let ret = boxed_future(out);

        let call = quote! { Self::#mangled(self, #(#names),*).await };

        let body = if *ident == WRAPPED_ITEM {
            // Site 2. Which items are per-session, and which `NewSessionResponse`
            // field advertises each, are BOTH answered by plexus-acp — this
            // crate matches on the payload it is handed, and fails closed on a
            // payload it does not recognise.
            quote! {
                fn __acp_err(message: ::std::string::String) -> #acp::v1::schema::Error {
                    let mut __e = #acp::v1::schema::Error::internal_error();
                    __e.message = message;
                    __e
                }
                fn __acp_advertises(
                    __derived: &#acp::v1::Derived,
                    __field: &str,
                ) -> bool {
                    __derived.per_session.iter().any(|__m| {
                        match #acp::v1::AgentMethod::requirement(*__m) {
                            #acp::v1::Requirement::Optional(
                                #acp::v1::Advertisement::PerSession(__f),
                            ) => __f == __field,
                            _ => false,
                        }
                    })
                }

                let __response = #call?;
                let __derived = Self::acp_derived();

                // Forward: an implemented per-session method whose session says
                // nothing. This is the exact shape of "implemented but never
                // advertised" that PLX-135 refused to drop.
                for __m in __derived.per_session.iter().copied() {
                    let #acp::v1::Requirement::Optional(
                        #acp::v1::Advertisement::PerSession(__field),
                    ) = #acp::v1::AgentMethod::requirement(__m) else {
                        continue;
                    };
                    let __present = match __field {
                        "modes" => __response.modes.is_some(),
                        "configOptions" => __response.config_options.is_some(),
                        __other => {
                            return ::core::result::Result::Err(__acp_err(::std::format!(
                                "plexus-acp advertises `{}` on a `NewSessionResponse` \
                                 field `{}` that #[acp_agent] does not know how to \
                                 check; refusing the session rather than letting an \
                                 implemented method go unadvertised",
                                #acp::v1::AgentMethod::trait_item(__m),
                                __other,
                            )));
                        }
                    };
                    if !__present {
                        return ::core::result::Result::Err(__acp_err(::std::format!(
                            "`{}` is implemented, so every session that supports it \
                             MUST advertise it on `NewSessionResponse.{}` — that field \
                             is unset. Implementing a method and never advertising it \
                             is the failure #[acp_agent] exists to prevent",
                            #acp::v1::AgentMethod::trait_item(__m),
                            __field,
                        )));
                    }
                }

                // Reverse: a session offering a choice the agent cannot service.
                if !__acp_advertises(&__derived, "modes") {
                    if let ::core::option::Option::Some(ref __modes) = __response.modes {
                        if __modes.available_modes.len() > 1 {
                            return ::core::result::Result::Err(__acp_err(
                                ::std::string::ToString::to_string(
                                    "this session advertises more than one mode but \
                                     `set_session_mode` is not implemented, so the \
                                     client could never switch to any of them",
                                ),
                            ));
                        }
                    }
                }
                if !__acp_advertises(&__derived, "configOptions") {
                    if let ::core::option::Option::Some(ref __opts) = __response.config_options {
                        if !__opts.is_empty() {
                            return ::core::result::Result::Err(__acp_err(
                                ::std::string::ToString::to_string(
                                    "this session advertises config options but \
                                     `set_session_config_option` is not implemented, so \
                                     the client could never set any of them",
                                ),
                            ));
                        }
                    }
                }

                ::core::result::Result::Ok(__response)
            }
        } else {
            quote! { #call }
        };

        wrappers.push(quote! {
            fn #ident<'__acp_life0, '__acp_async_trait>(
                &'__acp_life0 self,
                #(#args),*
            ) -> #ret
            where
                '__acp_life0: '__acp_async_trait,
                Self: '__acp_async_trait,
            {
                ::std::boxed::Box::pin(async move { #body })
            }
        });
    }

    Ok(quote! {
        #[automatically_derived]
        impl #impl_generics #self_ty #where_clause {
            #(#author_items)*
        }

        #derived_impl

        #[automatically_derived]
        #[allow(clippy::used_underscore_items)]
        impl #impl_generics #acp::v1::Agent for #self_ty #where_clause {
            #initialize_fn
            #(#wrappers)*
        }

    })
}
