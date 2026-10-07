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
//! For `#[plexus_macros::activation(client)] impl Tickets` (namespace
//! `tickets`), beside the activation's own items:
//!
//! ```ignore
//! /// The aggregate rule (RETURNS-1 decisions 1, 5; pin 13).
//! #[derive(Debug, Clone, Serialize, Deserialize)]
//! #[serde(tag = "method", content = "event")]
//! pub enum TicketsEvent {
//!     #[serde(rename = "health")] Health(Health),
//!     #[serde(rename = "get")]    Get(#[serde(with = "plexus_client::wire_result")] Result<Ticket, GetError>),
//!     #[serde(rename = "list")]   List(#[serde(with = "plexus_client::wire_result")] Result<ListItem, ListError>),
//!     #[serde(rename = "import")] Import(#[serde(with = "plexus_client::wire_result")] Result<ImportSummary, ImportError>),
//! }
//!
//! pub struct TicketsClient<T> { transport: T }
//!
//! impl<T> TicketsClient<T> {
//!     pub const NAMESPACE: &'static str = "tickets";
//!     pub fn new(transport: T) -> Self;
//!     pub fn transport(&self) -> &T;
//!     pub fn into_transport(self) -> T;
//! }
//!
//! impl<T: plexus_client::Transport> TicketsClient<T> {
//!     pub const METHOD_GET: plexus_client::MethodRef = ..;          // one per method
//!     pub async fn health(&self) -> Result<Health, ClientError>;     // one
//!     pub async fn get(&self, id: String)
//!         -> Result<Result<Ticket, GetError>, ClientError>;          // one, fallible
//!     pub fn list(&self, epic: Option<String>)                       // many
//!         -> MethodStream<'_, Result<ListItem, ListError>, NoProgress, TicketsEvent>;
//!     pub fn import(&self, source: String)                           // many, with progress
//!         -> MethodStream<'_, Result<ImportSummary, ImportError>, ImportStep, TicketsEvent>;
//! }
//! ```
//!
//! | method form (server) | client method | yields |
//! |---|---|---|
//! | one: `-> T` / `-> Result<T, E>` | `async fn m(&self, ..) -> Result<I, ClientError>` | `I` = `T` / `Result<T, E>` |
//! | many: `-> impl Stream<Item = …>` | `fn m(&self, ..) -> MethodStream<'_, I, P, {Ns}Event>` | `Result<I, ClientError>`; `.with_progress()`, `.events()` |
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
//! **The aggregate derives through `plexus_client::__rt::serde`**
//! (`#[serde(crate = "…")]`), so the emitted code needs nothing in the
//! consumer's `Cargo.toml` beyond `plexus-client`. It derives `Debug` and
//! `Clone`, so every item type of a `client` activation implements both.

use std::collections::HashMap;

use crate::parse::{Cardinality, MethodInfo, ParamInfo};
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
fn param_link(param: &ParamInfo) -> TokenStream {
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
fn method_const_ident(m: &MethodInfo) -> syn::Ident {
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

/// Emit the whole client for one activation: the `{Ns}Event` aggregate, the
/// `{Struct}Client<T>` and its methods.
pub(crate) fn generate(
    struct_name: &syn::Ident,
    namespace: &str,
    methods: &[MethodInfo],
    client_path: &syn::Path,
) -> syn::Result<TokenStream> {
    let client_ty = format_ident!("{}Client", struct_name);
    let event_ty = derived_ident(
        &aggregate_name(namespace),
        &format!("the namespace `{namespace}`"),
        Span::call_site(),
    )?;
    let wire_result_path = path_string(quote! { #client_path::wire_result });
    let serde_path = path_string(quote! { #client_path::__rt::serde });

    let mut variants: Vec<TokenStream> = Vec::new();
    let mut method_items: Vec<TokenStream> = Vec::new();
    let mut consts: Vec<TokenStream> = Vec::new();
    let mut seen: HashMap<String, String> = HashMap::new();

    for m in methods {
        let wire_method = &m.method_name;

        // ---------------------------------------------- the aggregate variant
        let variant_name = pascal(wire_method);
        if let Some(other) = seen.insert(variant_name.clone(), wire_method.clone()) {
            return Err(syn::Error::new(
                m.fn_name.span(),
                format!(
                    "#[plexus_macros::activation(client)]: methods `{other}` and \
                     `{wire_method}` both derive the `{event_ty}::{variant_name}` \
                     variant of the activation's aggregate; rename one of them"
                ),
            ));
        }
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
                quote! { #client_path::__rt::serde_json::from_value::<#ok_ty> },
                quote! { #ok_ty },
            ),
            Some(err_ty) => (
                quote! { ::core::result::Result<#ok_ty, #err_ty> },
                quote! { #client_path::wire_result::from_value::<#ok_ty, #err_ty> },
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
                        #client_path::call_one(
                            &self.transport,
                            Self::#const_ident,
                            #params,
                            #decoder,
                        )
                        .await
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
                     `.events()` yields `{event_ty}::{variant}`."
                );
                quote! {
                    #[allow(clippy::too_many_arguments)]
                    #[doc = #doc]
                    pub fn #fn_ident(&self #(, #sig_params)*)
                        -> #client_path::MethodStream<'_, #item_ty, #progress_ty, #event_ty>
                    {
                        #client_path::MethodStream::call(
                            &self.transport,
                            Self::#const_ident,
                            #params,
                            #decoder,
                            #event_ty::#variant,
                        )
                    }
                }
            }
        };
        method_items.push(method);
    }

    let event_doc = format!(
        "The `{namespace}` activation's event: one variant per method, keyed by \
         the method's wire name, carrying that method's item — derived by \
         `#[plexus_macros::activation(client)]`, never sent on the wire.\n\n\
         Wire shape `{{\"method\": \"<name>\", \"event\": <item>}}`, a Result \
         item as `{{\"ok\"}}` / `{{\"err\"}}`. A many-item call's `.events()` \
         yields it; merging several calls into one listener is ordinary stream \
         plumbing. Progress and call failures are not items, so they are not \
         here."
    );

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

    Ok(quote! {
        #[doc = #event_doc]
        #[derive(
            ::core::fmt::Debug,
            ::core::clone::Clone,
            #client_path::__rt::serde::Serialize,
            #client_path::__rt::serde::Deserialize,
        )]
        #[serde(crate = #serde_path, tag = "method", content = "event")]
        pub enum #event_ty {
            #(#variants)*
        }

        #[doc = #struct_doc]
        #[derive(Debug, Clone, Copy, Default)]
        pub struct #client_ty<T> {
            transport: T,
        }

        impl<T> #client_ty<T> {
            /// The activation namespace this client speaks to.
            pub const NAMESPACE: &'static str = #namespace;

            /// Wrap a transport.
            pub fn new(transport: T) -> Self {
                Self { transport }
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
            #(#method_items)*
        }
    })
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
