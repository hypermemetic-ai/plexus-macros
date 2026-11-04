//! The typed client half of `#[plexus_macros::activation]` (MACRO-2; 0.7
//! shapes RETURNS-9).
//!
//! The macro already knows, at the moment it runs, everything a Rust
//! consumer needs to call the activation it is expanding: every
//! `#[method]`'s wire name, its parameters (names, types, optionality) and
//! its precise contract — cardinality, item `T`, error `E`, progress `P`
//! (RETURNS-8's `MethodInfo`). That knowledge goes into the schema and, with
//! the `client` flag, into a client, so no consumer ever hand-writes a call
//! site or hand-mirrors an item type.
//!
//! # Shape of the emitted code
//!
//! For `#[plexus_macros::activation] impl TicketsActivation` (namespace
//! `tickets`; MACRO-13: emitted whenever the crate depends on
//! plexus-client), beside the activation's own items:
//!
//! ```ignore
//! /// The aggregate rule (RETURNS-1 decisions 1, 5; pin 13).
//! #[derive(Debug, Serialize, Deserialize)]
//! #[serde(tag = "method", content = "event")]
//! pub enum TicketsEvent {
//!     #[serde(rename = "health")] Health(Health),
//!     #[serde(rename = "get")]    Get(#[serde(with = "plexus_client::wire_result")] Result<Ticket, GetError>),
//!     #[serde(rename = "list")]   List(#[serde(with = "plexus_client::wire_result")] Result<ListItem, ListError>),
//!     #[serde(rename = "import")] Import(#[serde(with = "plexus_client::wire_result")] Result<ImportSummary, ImportError>),
//!     // RETURNS-46: one variant per #[child], carrying the child's aggregate
//!     #[serde(rename = "inputs")] Inputs(<InputsClient<()> as plexus_client::Aggregated>::Event),
//!     #[serde(rename = "item")]   Item { name: Arc<str>, event: Box<<ItemClient<()> as plexus_client::Aggregated>::Event> },
//! }
//! impl From<InputsEvent> for TicketsEvent { .. }                 // one per static child
//!
//! pub struct TicketsClient<T, L = Root> { transport: T, route: Route, lift: L }
//!
//! impl<T> TicketsClient<T> {
//!     pub const NAMESPACE: &'static str = "tickets";
//!     pub fn new(transport: T) -> Self;
//!     pub fn at(transport: T, path: impl Into<RoutePath>) -> Self;   // MACRO-16
//!     pub const METHOD_GET: plexus_client::MethodRef = ..;   // one per method (T: Transport)
//! }
//!
//! impl<T, L> TicketsClient<T, L> {
//!     pub fn lifted(transport: T, route: Route, lift: L) -> Self;  // RETURNS-46
//!     pub fn path(&self) -> &str;
//!     pub fn transport(&self) -> &T;
//!     pub fn into_transport(self) -> T;
//! }
//!
//! impl<T: plexus_client::Transport, L: Lift<TicketsEvent>> TicketsClient<T, L> {
//!     pub async fn health(&self) -> Result<Health, ClientError>;     // one
//!     pub async fn get(&self, id: String)
//!         -> Result<Result<Ticket, GetError>, ClientError>;          // one, fallible
//!     pub fn list(&self, epic: Option<String>)                       // many
//!         -> MethodStream<'_, Result<ListItem, ListError>, NoProgress, TicketsEvent, L>;
//!     pub fn import(&self, source: String)                           // many, with progress
//!         -> MethodStream<'_, Result<ImportSummary, ImportError>, ImportStep, TicketsEvent, L>;
//!     pub fn inputs(&self) -> InputsClient<&T, Static<L, InputsEvent, TicketsEvent>>;   // a static #[child]
//!     pub fn item(&self, name: &str) -> ItemClient<&T, Named<L, ItemEvent, TicketsEvent>>;  // a dynamic #[child]
//! }
//! ```
//!
//! | method form (server) | client method | yields |
//! |---|---|---|
//! | one: `-> T` / `-> Result<T, E>` | `async fn m(&self, ..) -> Result<I, ClientError>` | `I` = `T` / `Result<T, E>` |
//! | many: `-> impl Stream<Item = …>` | `fn m(&self, ..) -> MethodStream<'_, I, P, {Ns}Event, L>` (`.events()` yields `L::Out`: `{Ns}Event` unless reached through a parent) | `Result<I, ClientError>`; `.with_progress()`, `.events()` |
//!
//! `P` is the declared progress type (`Note` for `Progress<T>`), or
//! `plexus_client::NoProgress` when the item is not a `Progress`. A one-item
//! method has no progress (RETURNS-1 decision 10), so it is a plain `async
//! fn` and has no `.with_progress()`.
//!
//! # Decisions worth knowing
//!
//! **Emitted at the impl block's own scope, not inside a module.** A module
//! would need `use super::*` to reach the activation's item and parameter
//! types, and a glob import is exactly the thing that breaks in a consumer's
//! crate for reasons the consumer cannot see.
//!
//! **Generic over the transport.** The transports have irreconcilable
//! dependency sets (jsonrpsee's ws-client, an HTTP stack, `plexus-core`
//! itself). Naming one here would drag it into every consumer, including the
//! wasm32 one. `T: Transport` costs one type parameter and keeps the emitted
//! code free of all of them.
//!
//! **`Option` parameters are omitted, not nulled.** menger's `Ticket`
//! serializes `severity`/`parent` with `skip_serializing_if`, and its REST
//! tests assert absence rather than null. A client that sent `"parent":
//! null` would be sending a shape the server never sends.
//!
//! **The aggregate nests up the tree, and a parent's client lifts**
//! (RETURNS-46). A child's variant carries the child's aggregate, named
//! through its client (`<InputsClient<()> as Aggregated>::Event`) — the
//! client's name is the one thing the parent knows (the stem of the
//! accessor's return type); the aggregate is named by the child's
//! namespace, which may differ. A dynamic child's variant also carries the
//! instance's name (`Arc<str>`) and boxes its event: every servable cycle
//! passes through a dynamic child, so that breaks them all while a static
//! level allocates nothing. `-> Self` is the recursive case, boxed.
//! Lifting is the client's, and a type: `XClient<T, L = Root>`, `L` one of
//! `Root`, `Static`, `Named`; an accessor wraps its variant onto its own
//! lift, so a call anywhere below a client yields that client's aggregate —
//! the only form that reaches a grandchild in one step and puts a dynamic
//! instance's name in its events without the caller repeating it. `From`
//! per static child is the one-level form for a child's own client. A
//! `#[child(opaque)]` is left out, and a child without a client is held to
//! `HasClient` in the server half, whose message says to add `opaque`.
//!
//! **The aggregate derives through `plexus_client::__rt::serde`**
//! (`#[serde(crate = "…")]`), so the emitted code needs nothing in the
//! consumer's `Cargo.toml` beyond `plexus-client`. It derives `Debug`, so
//! every item type of an activation with a client implements it. Not `Clone`
//! (RETURNS-36): under MACRO-13 the client is emitted whenever the crate
//! depends on plexus-client, so the aggregate asks no more of an existing
//! activation's item types than its client does.
//!
//! **The aggregate's name is `{Ns}Event`, unless an item type of the
//! activation is already called that** (RETURNS-36): an activation written
//! before 0.7 yields one shared event enum, usually named so (`EchoEvent`
//! for `echo`), and under MACRO-13 its client — and so the aggregate — is
//! emitted without asking. Then the aggregate is `{Ns}Aggregate`.

use std::collections::HashMap;

use crate::parse::{Cardinality, ChildMethodInfo, ChildMethodKind, MethodInfo, ParamInfo};
use proc_macro2::{Span, TokenStream};
use proc_macro_crate::{crate_name, FoundCrate};
use quote::{format_ident, quote};
use syn::Type;

/// Resolve the path to `plexus-client` in the *consuming* crate's manifest.
///
/// Absence is an error rather than a silent skip: the `client` flag is an
/// explicit opt-in, so a missing dependency is a mistake the author wants
/// named at the attribute, not as an unresolved import inside code they
/// never wrote.
pub(crate) fn resolve_client_path(err_span: Span) -> syn::Result<syn::Path> {
    match crate_name("plexus-client") {
        Ok(FoundCrate::Itself) => Ok(syn::parse_quote!(crate)),
        Ok(FoundCrate::Name(name)) => {
            let ident = syn::Ident::new(&name, err_span);
            Ok(syn::parse_quote!(::#ident))
        }
        // The plexus-rpc umbrella does not re-export plexus-client yet; when
        // it does, this is where `::<umbrella>::client` goes.
        Err(e) => Err(syn::Error::new(
            err_span,
            format!(
                "#[plexus_macros::activation(client)] could not locate the \
                 `plexus-client` dependency in Cargo.toml ({e}). The generated \
                 client links against it. Add `plexus-client` as a dependency, \
                 or remove the `client` flag."
            ),
        )),
    }
}

/// `true` for `Option<..>` written any way a consumer plausibly writes it.
///
/// Matching on the last path segment accepts `Option<T>`,
/// `std::option::Option<T>` and `core::option::Option<T>`. A type *aliased*
/// to `Option` is not recognised — it would be sent as `null` instead of
/// omitted. That is the same limitation serde's own `skip_serializing_if`
/// convention has, and it fails visibly (a `null` on the wire) rather than
/// silently.
fn is_option_type(ty: &Type) -> bool {
    let Type::Path(p) = ty else { return false };
    p.qself.is_none()
        && p.path
            .segments
            .last()
            .is_some_and(|s| s.ident == "Option" && matches!(s.arguments, syn::PathArguments::AngleBracketed(_)))
}

/// One parameter's link in the `Params` builder chain.
pub(crate) fn param_link(param: &ParamInfo) -> TokenStream {
    let ident = &param.name;
    let wire_name = param.name.to_string();
    if is_option_type(&param.ty) {
        // Omitted when None — see the module docs.
        quote! { .optional(#wire_name, #ident) }
    } else {
        quote! { .param(#wire_name, #ident) }
    }
}

/// `METHOD_LIST`, `METHOD_SET_STATUS`, … — the const `MethodRef` a client
/// method hands the transport.
pub(crate) fn method_const_ident(m: &MethodInfo) -> syn::Ident {
    format_ident!("METHOD_{}", m.method_name.to_uppercase().replace(['.', '-'], "_"))
}

/// PascalCase of a wire name: `get_status` → `GetStatus`, `claude_code` →
/// `ClaudeCode`. Words split at `_`, `-` and `.`; each word's first letter is
/// uppercased and the rest kept as written.
fn pascal(name: &str) -> String {
    name.split(['_', '-', '.'])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect::<String>(),
                None => String::new(),
            }
        })
        .collect()
}

/// `name` as an identifier, or an error naming what it was derived from.
fn derived_ident(name: &str, what: &str, span: Span) -> syn::Result<syn::Ident> {
    syn::parse_str::<syn::Ident>(name)
        .map(|mut ident| {
            ident.set_span(span);
            ident
        })
        .map_err(|_| {
            syn::Error::new(
                span,
                format!(
                    "#[plexus_macros::activation(client)]: {what} derives the \
                     identifier `{name}`, which is not a valid Rust identifier"
                ),
            )
        })
}

/// The aggregate's name: Pascal(namespace) + `Event` (RETURNS-1 decision 7).
pub(crate) fn aggregate_name(namespace: &str) -> String {
    format!("{}Event", pascal(namespace))
}

/// A path as the string a serde attribute takes (`crate = "…"`, `with =
/// "…"`), without the spaces `TokenStream`'s `Display` puts between tokens.
fn path_string(tokens: TokenStream) -> String {
    tokens.to_string().replace(' ', "")
}

/// The aggregate's name, given the activation's methods: `{Ns}Event`, or
/// `{Ns}Aggregate` when an item type of a method is already named
/// `{Ns}Event` (see the module docs).
pub(crate) fn aggregate_name_for(namespace: &str, methods: &[MethodInfo]) -> String {
    let name = aggregate_name(namespace);
    let named = |ty: &Type| match ty {
        Type::Path(tp) => tp.path.segments.last().is_some_and(|s| s.ident == name),
        _ => false,
    };
    if methods.iter().any(|m| named(&m.ok_ty) || m.err_ty.as_ref().is_some_and(named)) {
        format!("{}Aggregate", pascal(namespace))
    } else {
        name
    }
}

/// Emit the whole client for one activation: the `{Ns}Event` aggregate, the
/// client struct (`{Stem}Client<T>`, MACRO-13's name), its methods and its
/// static children's accessors (MACRO-16).
pub(crate) fn generate(
    client_ty: &syn::Ident,
    namespace: &str,
    methods: &[MethodInfo],
    children: &[ChildMethodInfo],
    client_path: &syn::Path,
) -> syn::Result<(TokenStream, TokenStream)> {
    let event_ty = derived_ident(
        &aggregate_name_for(namespace, methods),
        &format!("the namespace `{namespace}`"),
        Span::call_site(),
    )?;
    // RETURNS-46: a `#[child(opaque)]` has no client — no variant, no
    // accessor. Every child is a segment an instance name must not be.
    let tree = children
        .iter()
        .filter(|c| !c.opaque)
        .map(|c| ChildNode::of(c, client_ty, &event_ty, client_path))
        .collect::<syn::Result<Vec<ChildNode>>>()?;
    let reserved = Reserved {
        statics: children
            .iter()
            .filter(|c| matches!(c.kind, ChildMethodKind::Static))
            .map(|c| c.fn_name.to_string())
            .collect(),
        methods: methods.iter().map(|m| m.method_name.clone()).chain(["schema".to_string()]).collect(),
    };
    let child_accessors: Vec<TokenStream> =
        tree.iter().map(|n| child_accessor(n, namespace, &event_ty, client_path, &reserved)).collect();
    // RETURNS-46: in the server half, where the child's type exists, each
    // child is held to having a client — so a child without one is told to
    // be `#[child(opaque)]`, not only that `XClient` is not found.
    let server_checks: Vec<TokenStream> = tree
        .iter()
        .filter(|n| !n.recursive)
        .map(|n| {
            let ty = &n.child_ty;
            quote! {
                const _: fn() = || {
                    fn __plexus_child_needs_a_client<C: #client_path::HasClient>() {}
                    __plexus_child_needs_a_client::<#ty>();
                };
            }
        })
        .collect();
    let wire_result_path = path_string(quote! { #client_path::wire_result });
    let serde_path = path_string(quote! { #client_path::__rt::serde });

    let mut variants: Vec<TokenStream> = Vec::new();
    let mut method_items: Vec<TokenStream> = Vec::new();
    let mut consts: Vec<TokenStream> = Vec::new();
    let mut seen: HashMap<String, String> = HashMap::new();
    // RETURNS-46: methods and children share the route namespace, and so
    // the aggregate's variants (and their wire tags): one name, one variant.
    let mut claim = |variant_name: &str, what: String, span: Span| -> syn::Result<()> {
        match seen.insert(variant_name.to_string(), what.clone()) {
            None => Ok(()),
            Some(other) => Err(syn::Error::new(
                span,
                format!(
                    "#[plexus_macros::activation(client)]: {other} and {what} both derive \
                     the `{event_ty}::{variant_name}` variant of the activation's aggregate \
                     (methods and children share one route namespace); rename one of them"
                ),
            )),
        }
    };

    for m in methods {
        let wire_method = &m.method_name;

        // ---------------------------------------------- the aggregate variant
        let variant_name = pascal(wire_method);
        claim(&variant_name, format!("the method `{wire_method}`"), m.fn_name.span())?;
        let variant = derived_ident(
            &variant_name,
            &format!("the method `{wire_method}`"),
            m.fn_name.span(),
        )?;

        // I = T, or Result<T, E> with its {"ok"} / {"err"} wire form.
        let ok_ty = &m.ok_ty;
        let (item_ty, decoder, payload) = match &m.err_ty {
            None => (
                quote! { #ok_ty },
                quote! { #client_path::Content::decode::<#ok_ty> },
                quote! { #ok_ty },
            ),
            Some(err_ty) => (
                quote! { ::core::result::Result<#ok_ty, #err_ty> },
                quote! { #client_path::wire_result::from_content::<#ok_ty, #err_ty> },
                quote! {
                    #[serde(with = #wire_result_path)]
                    ::core::result::Result<#ok_ty, #err_ty>
                },
            ),
        };

        let variant_doc = format!("An item of `{namespace}.{wire_method}`.");
        variants.push(quote! {
            #[doc = #variant_doc]
            #[serde(rename = #wire_method)]
            #variant(#payload),
        });

        // ------------------------------------------------- the client method
        let fn_ident = &m.fn_name;
        let const_ident = method_const_ident(m);

        // Parameters, in signature order, minus the ones the server injects:
        // `#[activation_param]`, `#[from_auth]` and `auth:`/`ctx:` are
        // already absent from `MethodInfo::params` by construction, which is
        // exactly the set a caller must not be asked for.
        let sig_params: Vec<TokenStream> = m
            .params
            .iter()
            .map(|p| {
                let name = &p.name;
                let ty = &p.ty;
                quote! { #name: #ty }
            })
            .collect();
        let links: Vec<TokenStream> = m.params.iter().map(param_link).collect();
        let params = quote! { #client_path::Params::new() #(#links)* .build() };

        let calls = format!("Calls `{namespace}.{wire_method}`.");
        let doc = if m.description.is_empty() {
            calls
        } else {
            format!("{}\n\n{calls}", m.description)
        };

        consts.push(quote! {
            #[doc = #doc]
            pub const #const_ident: #client_path::MethodRef =
                #client_path::MethodRef::new(#namespace, #wire_method);
        });

        // The parameter list is the activation's, not ours: menger's
        // `create` takes seven. A lint the consumer cannot act on without
        // changing the *server* has no business firing in the crate that
        // merely opted into a client.
        let method = match m.cardinality {
            Cardinality::One => {
                let doc = format!(
                    "{doc}\n\nOne item: `Ok(item)` is the activation's answer; `Err` is \
                     the call failing (`ClientError::Remote` for the server's typed \
                     error frame)."
                );
                quote! {
                    #[allow(clippy::too_many_arguments)]
                    #[doc = #doc]
                    pub async fn #fn_ident(&self #(, #sig_params)*)
                        -> ::core::result::Result<#item_ty, #client_path::ClientError>
                    {
                        let (__method, __params) = match self.route.method(#client_ty::<T>::#const_ident) {
                            ::core::result::Result::Ok(m) => (m, #params),
                            ::core::result::Result::Err(e) => (#client_ty::<T>::#const_ident, ::core::result::Result::Err(e)),
                        };
                        #client_path::call_one(&self.transport, __method, __params, #decoder).await
                    }
                }
            }
            Cardinality::Many => {
                let progress_ty = match &m.progress_ty {
                    Some(p) => {
                        let p = p.to_type(client_path);
                        quote! { #p }
                    }
                    None => quote! { #client_path::NoProgress },
                };
                let doc = format!(
                    "{doc}\n\nMany items, as a stream of `Result<item, ClientError>`; \
                     progress is dropped unless opted in with `.with_progress()`, and \
                     `.events()` yields `{event_ty}::{variant}` — lifted by `L` into the \
                     aggregate of the client this one was reached from (RETURNS-46)."
                );
                quote! {
                    #[allow(clippy::too_many_arguments)]
                    #[doc = #doc]
                    pub fn #fn_ident(&self #(, #sig_params)*)
                        -> #client_path::MethodStream<'_, #item_ty, #progress_ty, #event_ty, L>
                    {
                        let (__method, __params) = match self.route.method(#client_ty::<T>::#const_ident) {
                            ::core::result::Result::Ok(m) => (m, #params),
                            ::core::result::Result::Err(e) => (#client_ty::<T>::#const_ident, ::core::result::Result::Err(e)),
                        };
                        #client_path::MethodStream::call(&self.transport, __method, __params, #decoder, #event_ty::#variant)
                            .lifted(::core::clone::Clone::clone(&self.lift))
                    }
                }
            }
        };
        method_items.push(method);
    }

    // RETURNS-46: one variant per child, after the methods', carrying the
    // child's aggregate — and a dynamic child's, the instance's name.
    let mut froms: Vec<TokenStream> = Vec::new();
    for n in &tree {
        let variant_name = pascal(&n.segment);
        let kind = if n.dynamic { "dynamic child" } else { "child" };
        claim(&variant_name, format!("the {kind} `{}`", n.segment), n.fn_name.span())?;
        let variant = &n.variant;
        let segment = &n.segment;
        let payload = &n.payload;
        if n.dynamic {
            let doc = format!(
                "An event of an instance of the dynamic child `{segment}` (`{namespace}.<name>`): \
                 the instance's name, and its aggregate."
            );
            variants.push(quote! {
                #[doc = #doc]
                #[serde(rename = #segment)]
                #variant {
                    /// The instance's name: its route segment, shared by
                    /// every event of the instance.
                    name: ::std::sync::Arc<str>,
                    /// The instance's event, in its own aggregate.
                    event: #payload,
                },
            });
        } else {
            let doc = format!("An event of the child `{namespace}.{segment}`, in its aggregate.");
            variants.push(quote! {
                #[doc = #doc]
                #[serde(rename = #segment)]
                #variant(#payload),
            });
            // A child's own aggregate lifts into this one by `From`, unless
            // two static children share an aggregate (which variant would it
            // be?) or the child is this activation itself.
            let shared = tree.iter().filter(|o| !o.dynamic && o.child_client_key == n.child_client_key).count() > 1;
            if !shared && !n.recursive {
                let child_event = &n.child_event;
                let wrapped = if n.boxed { quote! { ::std::boxed::Box::new(event) } } else { quote! { event } };
                let doc = format!("`{event_ty}::{variant}`: the child `{segment}`'s event, lifted (RETURNS-46).");
                froms.push(quote! {
                    #[doc = #doc]
                    impl ::core::convert::From<#child_event> for #event_ty {
                        fn from(event: #child_event) -> Self {
                            #event_ty::#variant(#wrapped)
                        }
                    }
                });
            }
        }
    }

    let event_doc = format!(
        "The `{namespace}` activation's event: one variant per method, keyed by \
         the method's wire name, carrying that method's item, then one per \
         `#[child]`, keyed by its route segment, carrying the child's own \
         aggregate — derived by `#[plexus_macros::activation(client)]`, never \
         sent on the wire.\n\n\
         Wire shape `{{\"method\": \"<name>\", \"event\": <item>}}`, a Result \
         item as `{{\"ok\"}}` / `{{\"err\"}}`; a static child's event as \
         `{{\"method\": \"<child>\", \"event\": <child's aggregate>}}`, a dynamic \
         child's as `{{\"method\": \"<child>\", \"event\": {{\"name\": \"<instance>\", \
         \"event\": <child's aggregate>}}}}` (RETURNS-46). A many-item call's \
         `.events()` yields it — from a call anywhere below a client, that \
         client's aggregate; merging several calls into one listener is \
         ordinary stream plumbing. Progress and call failures are not items, \
         so they are not here."
    );

    let client_name = client_ty.to_string();
    let struct_doc = format!(
        "Typed client for the `{namespace}` activation, generated by \
         `#[plexus_macros::activation(client)]`.\n\n\
         Generic over the transport: construct it with any \
         `plexus_client::Transport` — in-process for tests, WebSocket \
         JSON-RPC or REST + SSE against a server.\n\n\
         A one-item method is an `async fn` returning `Result<item, \
         ClientError>`; a many-item method returns a `MethodStream`. The \
         parameter and item types are the activation's own, so a change to \
         either is a compile error here, not a runtime surprise."
    );

    Ok((quote! {
        #[doc = #event_doc]
        #[derive(
            ::core::fmt::Debug,
            #client_path::__rt::serde::Serialize,
            #client_path::__rt::serde::Deserialize,
        )]
        #[serde(crate = #serde_path, tag = "method", content = "event")]
        // RETURNS-36: the aggregate is as public as the client, whatever
        // the activation's item types are (MACRO-13 emits it unasked).
        #[allow(private_interfaces)]
        pub enum #event_ty {
            #(#variants)*
        }

        #(#froms)*

        #[doc = #struct_doc]
        pub struct #client_ty<T, L = #client_path::Root> {
            transport: T,
            /// MACRO-16, RETURNS-46: the dotted path this activation is
            /// reached at (empty: its namespace), and a refusal when an
            /// instance name on the way cannot be routed.
            route: #client_path::Route,
            /// RETURNS-46: this activation's aggregate into the aggregate
            /// of the client this one was reached from (`Root`: its own).
            lift: L,
        }

        impl<T, L> #client_path::Aggregated for #client_ty<T, L> {
            type Event = #event_ty;
        }

        impl<T: ::core::fmt::Debug, L> ::core::fmt::Debug for #client_ty<T, L> {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                f.debug_struct(#client_name)
                    .field("transport", &self.transport)
                    .field("path", &self.path())
                    .finish()
            }
        }

        impl<T: ::core::clone::Clone, L: ::core::clone::Clone> ::core::clone::Clone for #client_ty<T, L> {
            fn clone(&self) -> Self {
                Self {
                    transport: ::core::clone::Clone::clone(&self.transport),
                    route: ::core::clone::Clone::clone(&self.route),
                    lift: ::core::clone::Clone::clone(&self.lift),
                }
            }
        }

        impl<T: ::core::default::Default> ::core::default::Default for #client_ty<T> {
            fn default() -> Self {
                Self::new(::core::default::Default::default())
            }
        }

        impl<T> #client_ty<T> {
            /// The activation namespace this client speaks to.
            pub const NAMESPACE: &'static str = #namespace;

            /// Wrap a transport.
            pub fn new(transport: T) -> Self {
                Self { transport, route: #client_path::Route::ROOT, lift: #client_path::Root }
            }

            /// Wrap a transport, for this activation mounted at `path` —
            /// the dotted route from the service, e.g. `"tickets.inputs"`
            /// for a `#[child]`. `new` is `at(transport, NAMESPACE)`. Its
            /// events are this activation's own aggregate; a parent's child
            /// accessor builds one whose events are the parent's.
            pub fn at(transport: T, path: impl ::core::convert::Into<#client_path::RoutePath>) -> Self {
                Self { transport, route: #client_path::Route::at(path, #namespace), lift: #client_path::Root }
            }
        }

        impl<T, L> #client_ty<T, L> {
            /// RETURNS-46: a client on `route` whose calls' `.events()` are
            /// lifted by `lift` — what a parent's child accessor builds
            /// (`TreeClient::branch()` lifts into `TreeEvent`).
            pub fn lifted(transport: T, route: #client_path::Route, lift: L) -> Self {
                Self { transport, route, lift }
            }

            /// The dotted path this client's calls go to: the namespace,
            /// or the child path it was built at.
            pub fn path(&self) -> &str {
                self.route.path(#namespace)
            }

            /// Borrow the underlying transport — for reading a
            /// `MemoryTransport`'s recorded calls, or sharing one transport
            /// across several activations' clients.
            pub fn transport(&self) -> &T {
                &self.transport
            }

            /// Unwrap the transport.
            pub fn into_transport(self) -> T {
                self.transport
            }
        }

        impl<T: #client_path::Transport> #client_ty<T> {
            #(#consts)*
        }

        impl<T: #client_path::Transport, L: #client_path::Lift<#event_ty>> #client_ty<T, L> {
            #(#method_items)*
            #(#child_accessors)*
        }
    }, quote! { #(#server_checks)* }))
}

/// RETURNS-46: what an instance name may not be — a static child's segment
/// or a method of the parent (the service routes those first).
struct Reserved {
    statics: Vec<String>,
    methods: Vec<String>,
}


/// RETURNS-46: what the client half knows of one `#[child]`: its route
/// segment, its client, its aggregate and how the parent's aggregate holds it.
struct ChildNode {
    fn_name: syn::Ident,
    description: String,
    /// The route segment of a static child; a dynamic child's variant tag.
    segment: String,
    dynamic: bool,
    /// The child's activation type as written (`Inputs`; `Self` when
    /// recursive) — what the server half holds to having a client.
    child_ty: Type,
    /// The child's client, `InputsClient` (keeping any module path).
    child_client: TokenStream,
    /// `child_client`'s tokens, to tell two children of one type apart.
    child_client_key: String,
    /// The child is this activation itself (`-> Self`, or its own name).
    recursive: bool,
    /// The child's aggregate: `<InputsClient<()> as Aggregated>::Event` — named
    /// through its client, by the rule the client is named by, never guessed
    /// from the type (a child's aggregate is named by its namespace).
    child_event: TokenStream,
    /// What the parent's variant carries: the child's event, boxed. Always
    /// boxed — two activations may be each other's children (ping has the
    /// static pong, pong the dynamic ping), a cycle no one expansion sees.
    payload: TokenStream,
    boxed: bool,
    variant: syn::Ident,
}

impl ChildNode {
    fn of(c: &ChildMethodInfo, client_ty: &syn::Ident, event_ty: &syn::Ident, client: &syn::Path) -> syn::Result<Self> {
        let dynamic = matches!(c.kind, ChildMethodKind::Dynamic);
        // A dynamic child returns `Option<Child>`; its client is the child's.
        let child_ty = if dynamic { option_inner(&c.return_ty).unwrap_or(&c.return_ty) } else { &c.return_ty };
        let what = if dynamic { "a dynamic #[child] must return `Option<Child>`" } else { "a static #[child] must return" };
        let Type::Path(tp) = child_ty else {
            return Err(syn::Error::new_spanned(
                &c.return_ty,
                format!(
                    "#[plexus_macros::activation(client)]: {what} its activation type by name \
                     (`-> Inputs`) so the client can reach it through `InputsClient`, or be \
                     marked `#[child(opaque)]`"
                ),
            ));
        };
        if tp.qself.is_some() {
            return Err(syn::Error::new_spanned(
                &c.return_ty,
                "#[plexus_macros::activation(client)]: a #[child]'s return type must be a plain path",
            ));
        }
        let segment = c.fn_name.to_string();
        let variant = derived_ident(&pascal(&segment), &format!("the child `{segment}`"), c.fn_name.span())?;
        // `-> Self` is this activation, as its own name is.
        let is_self = tp.path.is_ident("Self");
        let mut path = tp.path.clone();
        let (child_client, recursive) = if is_self {
            (quote! { #client_ty }, true)
        } else {
            let single = path.segments.len() == 1;
            let Some(last) = path.segments.last_mut() else {
                return Err(syn::Error::new_spanned(&c.return_ty, "#[child]: empty return type path"));
            };
            // MACRO-13: a child named `…Activation` has its client named from
            // the stem (`InputsActivation` → `InputsClient`), as its own
            // expansion names it.
            let child_name = last.ident.to_string();
            let stem = crate::codegen::contract::stem_of(&child_name).unwrap_or(&child_name);
            last.ident = format_ident!("{}Client", stem, span = last.ident.span());
            last.arguments = syn::PathArguments::None;
            let recursive = single && last.ident == *client_ty;
            (quote! { #path }, recursive)
        };
        let child_event = if recursive {
            quote! { #event_ty }
        } else {
            quote! { <#child_client<()> as #client::Aggregated>::Event }
        };
        let boxed = dynamic || recursive;
        let payload = if boxed { quote! { ::std::boxed::Box<#child_event> } } else { child_event.clone() };
        Ok(Self {
            fn_name: c.fn_name.clone(),
            description: c.description.clone(),
            child_client_key: child_client.to_string(),
            child_ty: child_ty.clone(),
            segment,
            dynamic,
            child_client,
            recursive,
            child_event,
            payload,
            boxed,
            variant,
        })
    }
}

/// `T` of `Option<T>`, written any way `is_option_type` accepts.
fn option_inner(ty: &Type) -> Option<&Type> {
    if !is_option_type(ty) {
        return None;
    }
    let Type::Path(p) = ty else { return None };
    let syn::PathArguments::AngleBracketed(args) = &p.path.segments.last()?.arguments else { return None };
    match args.args.first()? {
        syn::GenericArgument::Type(t) => Some(t),
        _ => None,
    }
}

/// The accessor a parent's client gets for one `#[child]`.
///
/// A static child (MACRO-16) — `fn inputs(&self) -> InputsClient<&T, Static<L, InputsEvent, TicketsEvent>>`,
/// whose calls go to `<this client's path>.inputs.<method>`, as the service
/// routes them. A dynamic child (RETURNS-46) — `fn item(&self, name: &str)
/// -> ItemClient<&T, Named<L, ItemEvent, TicketsEvent>>`, whose calls go to
/// `<this client's path>.<name>.<method>`: the service routes a name no
/// static child has to the dynamic child, so the name is the segment and the
/// accessor's own name is not on the wire. A name the service would route
/// elsewhere — empty, dotted, a static child's segment, a method of this
/// activation — makes a client every call of which fails at once
/// (`ClientError::InvalidName`); the accessor itself never fails.
///
/// Either way the child's client lifts its events into this client's
/// aggregate — wrapped in this level's variant (with the name, for a dynamic
/// child), then lifted as this client's are — so a call anywhere below a
/// client yields that client's aggregate.
///
/// The child's client is named from the accessor's return type: its last
/// path segment's stem with `Client` appended (`Inputs` → `InputsClient`,
/// keeping any module path), never from the child struct itself — a struct
/// that is server-only (`server_cfg`) does not exist in the client half. So
/// the child must have a client too, or be `#[child(opaque)]`.
fn child_accessor(
    n: &ChildNode,
    namespace: &str,
    event_ty: &syn::Ident,
    client: &syn::Path,
    reserved: &Reserved,
) -> TokenStream {
    let fn_name = &n.fn_name;
    let segment = &n.segment;
    let child_client = &n.child_client;
    let child_event = &n.child_event;
    let variant = &n.variant;
    if n.dynamic {
        let statics = &reserved.statics;
        let methods = &reserved.methods;
        let routes = format!(
            "The instance `name` of the dynamic child `{segment}` — its methods go to \
             `{namespace}.<name>.<method>` (or beneath wherever this client is mounted), and \
             its events are this client's, as `{event_ty}::{variant} {{ name, event }}`. A \
             name that is empty, holds a `.`, or is a static child's segment or a method of \
             `{namespace}` is routed elsewhere by the service: every call through it fails at \
             once with `ClientError::InvalidName`, and nothing is sent."
        );
        let doc = if n.description.is_empty() { routes } else { format!("{}\n\n{routes}", n.description) };
        quote! {
            #[doc = #doc]
            pub fn #fn_name(&self, name: &str)
                -> #child_client<&T, #client::Named<L, #child_event, #event_ty>>
            {
                #child_client::lifted(
                    &self.transport,
                    self.route.instance(#namespace, name, &[#(#statics),*], &[#(#methods),*]),
                    #client::Named::new(
                        ::core::clone::Clone::clone(&self.lift),
                        ::std::sync::Arc::from(name),
                        |name, event| #event_ty::#variant { name, event: ::std::boxed::Box::new(event) },
                    ),
                )
            }
        }
    } else {
        let joined = format!("{namespace}.{segment}");
        let wrapped = if n.boxed { quote! { ::std::boxed::Box::new(event) } } else { quote! { event } };
        let routes = format!(
            "The `{segment}` child — its methods go to `{namespace}.{segment}.<method>` (or \
             beneath wherever this client is mounted), and its events are this client's, as \
             `{event_ty}::{variant}`."
        );
        let doc = if n.description.is_empty() { routes } else { format!("{}\n\n{routes}", n.description) };
        quote! {
            #[doc = #doc]
            pub fn #fn_name(&self) -> #child_client<&T, #client::Static<L, #child_event, #event_ty>> {
                #child_client::lifted(
                    &self.transport,
                    self.route.child(#segment, #joined),
                    #client::Static::new(::core::clone::Clone::clone(&self.lift), |event| #event_ty::#variant(#wrapped)),
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pascal_case_of_wire_names() {
        assert_eq!(pascal("ping"), "Ping");
        assert_eq!(pascal("get_status"), "GetStatus");
        assert_eq!(pascal("claude_code"), "ClaudeCode");
        assert_eq!(pascal("claudecode"), "Claudecode");
        assert_eq!(pascal("set-status"), "SetStatus");
        assert_eq!(pascal("a.b"), "AB");
        assert_eq!(pascal("getHTTP"), "GetHTTP");
    }

    #[test]
    fn the_aggregate_is_pascal_namespace_plus_event() {
        assert_eq!(aggregate_name("conformance"), "ConformanceEvent");
        assert_eq!(aggregate_name("tickets"), "TicketsEvent");
        assert_eq!(aggregate_name("claude_code"), "ClaudeCodeEvent");
        assert_eq!(aggregate_name("hyperforge"), "HyperforgeEvent");
    }

    #[test]
    fn an_identifier_that_is_not_one_is_an_error() {
        assert!(derived_ident("Self", "x", Span::call_site()).is_err());
        assert!(derived_ident("9Lives", "x", Span::call_site()).is_err());
        assert!(derived_ident("GetStatus", "x", Span::call_site()).is_ok());
    }
}
