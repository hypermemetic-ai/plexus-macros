//! MACRO-2 · the typed client half of `#[plexus_macros::activation]`.
//!
//! The macro already knows, at the moment it runs, everything a Rust
//! consumer needs to call the activation it is expanding: every
//! `#[method]`'s wire name, its parameters (names, types, optionality) and
//! its `Stream<Item = E>` event type. Before this module that knowledge went
//! only into the schema; now it also produces a client, so no consumer ever
//! hand-writes a call site or hand-mirrors an event type.
//!
//! # Shape of the emitted code
//!
//! For `#[plexus_macros::activation(namespace = "tickets", client)] impl
//! Tickets`, beside the activation's own items:
//!
//! ```ignore
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
//!     pub const METHOD_LIST: plexus_client::MethodRef = ..;
//!     pub fn list(&self, epic: Option<String>, ..) -> EventStream<'_, TicketsEvent>;
//!     pub async fn list_all(&self, epic: Option<String>, ..)
//!         -> Result<Vec<TicketsEvent>, ClientError>;
//! }
//! ```
//!
//! # Three decisions worth knowing
//!
//! **Emitted at the impl block's own scope, not inside a module.** A module
//! would need `use super::*` to reach the activation's event and parameter
//! types, and a glob import is exactly the thing that breaks in a consumer's
//! crate for reasons the consumer cannot see.
//!
//! **Generic over the transport.** The three transports have irreconcilable
//! dependency sets (jsonrpsee's ws-client, an HTTP stack, `plexus-core`
//! itself). Naming one here would drag it into every consumer, including the
//! wasm32 one. `T: Transport` costs one type parameter and keeps the emitted
//! code free of all of them.
//!
//! **`Option` parameters are omitted, not nulled.** menger's `Ticket`
//! serializes `severity`/`parent` with `skip_serializing_if`, and its REST
//! tests assert absence rather than null. A client that sent `"parent":
//! null` would be sending a shape the server never sends.

use crate::parse::{MethodInfo, ParamInfo};
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

/// The statement that puts one parameter into the params object.
fn param_insert(param: &ParamInfo, client: &syn::Path) -> TokenStream {
    let ident = &param.name;
    let wire_name = param.name.to_string();

    if is_option_type(&param.ty) {
        // Omitted when None — see the module docs.
        quote! {
            if let ::core::option::Option::Some(__v) = #ident {
                match #client::__rt::serde_json::to_value(__v) {
                    ::core::result::Result::Ok(__json) => {
                        __params.insert(#wire_name.to_string(), __json);
                    }
                    ::core::result::Result::Err(__e) => {
                        return #client::error_stream(#client::ClientError::Encode {
                            param: #wire_name,
                            message: __e.to_string(),
                        });
                    }
                }
            }
        }
    } else {
        quote! {
            match #client::__rt::serde_json::to_value(#ident) {
                ::core::result::Result::Ok(__json) => {
                    __params.insert(#wire_name.to_string(), __json);
                }
                ::core::result::Result::Err(__e) => {
                    return #client::error_stream(#client::ClientError::Encode {
                        param: #wire_name,
                        message: __e.to_string(),
                    });
                }
            }
        }
    }
}

/// `METHOD_LIST`, `METHOD_SET_STATUS`, … — the const `MethodRef` a client
/// method hands the transport.
fn method_const_ident(m: &MethodInfo) -> syn::Ident {
    format_ident!("METHOD_{}", m.method_name.to_uppercase().replace(['.', '-'], "_"))
}

/// Emit the whole client for one activation.
///
/// Returns an empty token stream when the activation did not opt in, so a
/// non-opted-in expansion is byte-identical to before this build.
pub(crate) fn generate(
    struct_name: &syn::Ident,
    namespace: &str,
    methods: &[MethodInfo],
    client_path: &syn::Path,
) -> syn::Result<TokenStream> {
    let client_ty = format_ident!("{}Client", struct_name);

    let mut method_items: Vec<TokenStream> = Vec::new();
    let mut consts: Vec<TokenStream> = Vec::new();

    for m in methods {
        // A method whose return type is not `impl Stream<Item = E>` has no
        // event type to give a caller. Silently omitting it from the client
        // would recreate exactly the drift this ticket removes — a consumer
        // would find the method missing and hand-write it again.
        let Some(event_ty) = &m.stream_item_type else {
            return Err(syn::Error::new(
                m.fn_name.span(),
                format!(
                    "#[plexus_macros::activation(client)]: method `{}` does not return \
                     `impl Stream<Item = ..>`, so the generated client has no event type \
                     for it. Give it a stream return type, or remove the `client` flag \
                     from the activation.",
                    m.method_name
                ),
            ));
        };

        let fn_ident = &m.fn_name;
        let all_ident = format_ident!("{}_all", m.fn_name);
        let const_ident = method_const_ident(m);
        let wire_method = &m.method_name;

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
        let arg_names: Vec<&syn::Ident> = m.params.iter().map(|p| &p.name).collect();
        let inserts: Vec<TokenStream> = m
            .params
            .iter()
            .map(|p| param_insert(p, client_path))
            .collect();

        let doc = if m.description.is_empty() {
            format!("Calls `{namespace}.{wire_method}`.")
        } else {
            format!("{}\n\nCalls `{namespace}.{wire_method}`.", m.description)
        };
        let doc_all = format!(
            "{doc}\n\nDrives the stream to completion and collects every data event."
        );

        consts.push(quote! {
            #[doc = #doc]
            pub const #const_ident: #client_path::MethodRef =
                #client_path::MethodRef::new(#namespace, #wire_method);
        });

        method_items.push(quote! {
            // The parameter list is the activation's, not ours: menger's
            // `create` takes seven. A lint the consumer cannot act on
            // without changing the *server* has no business firing in the
            // crate that merely opted into a client.
            #[allow(clippy::too_many_arguments)]
            #[doc = #doc]
            pub fn #fn_ident(&self #(, #sig_params)*)
                -> #client_path::EventStream<'_, #event_ty>
            {
                let mut __params = #client_path::__rt::serde_json::Map::new();
                #(#inserts)*
                #client_path::decode::<#event_ty>(
                    #client_path::Transport::call(
                        &self.transport,
                        Self::#const_ident,
                        #client_path::__rt::serde_json::Value::Object(__params),
                    )
                )
            }

            #[allow(clippy::too_many_arguments)]
            #[doc = #doc_all]
            pub async fn #all_ident(&self #(, #sig_params)*)
                -> ::core::result::Result<
                    ::std::vec::Vec<#event_ty>,
                    #client_path::ClientError,
                >
            {
                #client_path::collect_data(self.#fn_ident(#(#arg_names),*)).await
            }
        });
    }

    let struct_doc = format!(
        "Typed client for the `{namespace}` activation, generated by \
         `#[plexus_macros::activation(client)]`.\n\n\
         Generic over the transport: construct it with any \
         `plexus_client::Transport` — in-process for tests, WebSocket \
         JSON-RPC or REST + SSE against a server.\n\n\
         The parameter and event types are the activation's own, so a change \
         to a method or an event is a compile error here, not a runtime \
         surprise."
    );

    Ok(quote! {
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
