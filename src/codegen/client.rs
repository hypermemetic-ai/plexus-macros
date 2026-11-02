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
//! pub struct TicketsClient<T> { transport: T, path: &'static str }
//!
//! impl<T> TicketsClient<T> {
//!     pub const NAMESPACE: &'static str = "tickets";
//!     pub fn new(transport: T) -> Self;
//!     pub fn at(transport: T, path: &'static str) -> Self;   // MACRO-16
//!     pub fn path(&self) -> &'static str;
//!     pub fn transport(&self) -> &T;
//!     pub fn into_transport(self) -> T;
//! }
//!
//! impl<T: plexus_client::Transport> TicketsClient<T> {
//!     pub const METHOD_LIST: plexus_client::MethodRef = ..;
//!     pub fn list(&self, epic: Option<String>, ..) -> EventStream<'_, TicketsEvent>;
//!     pub async fn list_all(&self, epic: Option<String>, ..)
//!         -> Result<Vec<TicketsEvent>, ClientError>;
//!     pub fn inputs(&self) -> InputsClient<&T>;   // a static #[child], MACRO-16
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

use crate::parse::{ChildMethodInfo, ChildMethodKind, MethodInfo, ParamInfo};
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
    param_insert_with(param, client, |err| quote! { return #client::error_stream(#err); })
}

/// [`param_insert`], with what to do when the parameter will not serialize
/// given by the caller (MACRO-13: a trait method returns a stream of events,
/// not of `Result`s, so it answers differently).
pub(crate) fn param_insert_with(
    param: &ParamInfo,
    client: &syn::Path,
    on_err: impl Fn(TokenStream) -> TokenStream,
) -> TokenStream {
    let ident = &param.name;
    let wire_name = param.name.to_string();
    let fail = on_err(quote! {
        #client::ClientError::Encode {
            param: #wire_name,
            message: __e.to_string(),
        }
    });

    if is_option_type(&param.ty) {
        // Omitted when None — see the module docs.
        quote! {
            if let ::core::option::Option::Some(__v) = #ident {
                match #client::__rt::serde_json::to_value(__v) {
                    ::core::result::Result::Ok(__json) => {
                        __params.insert(#wire_name.to_string(), __json);
                    }
                    ::core::result::Result::Err(__e) => {
                        #fail
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
                    #fail
                }
            }
        }
    }
}

/// `METHOD_LIST`, `METHOD_SET_STATUS`, … — the const `MethodRef` a client
/// method hands the transport.
pub(crate) fn method_const_ident(m: &MethodInfo) -> syn::Ident {
    format_ident!("METHOD_{}", m.method_name.to_uppercase().replace(['.', '-'], "_"))
}

/// Emit the whole client for one activation.
///
/// Returns an empty token stream when the activation did not opt in, so a
/// non-opted-in expansion is byte-identical to before this build.
pub(crate) fn generate(
    client_ty: &syn::Ident,
    namespace: &str,
    methods: &[MethodInfo],
    children: &[ChildMethodInfo],
    client_path: &syn::Path,
    // MACRO-13: the client came from the manifest, not from a `client` flag.
    unflagged: bool,
) -> syn::Result<TokenStream> {
    let child_accessors = children
        .iter()
        .filter(|c| matches!(c.kind, ChildMethodKind::Static))
        .map(|c| child_accessor(c, namespace, client_path))
        .collect::<syn::Result<Vec<TokenStream>>>()?;

    let mut method_items: Vec<TokenStream> = Vec::new();
    let mut consts: Vec<TokenStream> = Vec::new();

    for m in methods {
        // A method whose return type is not `impl Stream<Item = E>` has no
        // event type to give a caller. Silently omitting it from the client
        // would recreate exactly the drift this ticket removes — a consumer
        // would find the method missing and hand-write it again.
        let Some(event_ty) = &m.stream_item_type else {
            if unflagged {
                return Err(syn::Error::new(
                    m.fn_name.span(),
                    format!(
                        "#[plexus_macros::activation]: method `{}` does not return \
                         `impl Stream<Item = ..>`, so the client (emitted because this crate \
                         depends on plexus-client) has no event type for it. Give it a stream \
                         return type.",
                        m.method_name
                    ),
                ));
            }
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
                // PERF.115-36: the items with their content still the
                // wire's text, decoded straight into the event.
                #client_path::decode_raw::<#event_ty>(
                    #client_path::Transport::call_raw(
                        &self.transport,
                        Self::#const_ident.at(self.path),
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
            /// MACRO-16: the dotted path this activation is reached at;
            /// `""` for a hub's top-level activation (its namespace).
            path: &'static str,
        }

        impl<T> #client_ty<T> {
            /// The activation namespace this client speaks to.
            pub const NAMESPACE: &'static str = #namespace;

            /// Wrap a transport.
            pub fn new(transport: T) -> Self {
                Self { transport, path: "" }
            }

            /// Wrap a transport, for this activation mounted at `path` —
            /// the dotted route from the hub, e.g. `"tickets.inputs"` for a
            /// `#[child]`. A parent's child accessor builds this; `new` is
            /// `at(transport, NAMESPACE)`.
            pub fn at(transport: T, path: &'static str) -> Self {
                Self { transport, path: if path == Self::NAMESPACE { "" } else { path } }
            }

            /// The dotted path this client's calls go to: the namespace,
            /// or the child path it was built at.
            pub fn path(&self) -> &'static str {
                if self.path.is_empty() { Self::NAMESPACE } else { self.path }
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
            #(#child_accessors)*
        }
    })
}

/// MACRO-16: the accessor a parent's client gets for one static `#[child]`
/// — `fn inputs(&self) -> InputsClient<&T>`, whose calls go to
/// `<this client's path>.inputs.<method>`, as the hub routes them.
///
/// The child's client is named from the accessor's return type: its last
/// path segment with `Client` appended (`Inputs` → `InputsClient`, keeping
/// any module path), never from the child struct itself — a struct that is
/// server-only (`server_cfg`) does not exist in the client half. So the
/// child must be declared with `client` too; if it is not, the name does
/// not resolve and rustc points at the accessor's return type.
///
/// Dynamic children (`fn name(&self, id: &str) -> Option<Child>`) get no
/// accessor: their route segment is a runtime name. A recorded follow-up.
fn child_accessor(c: &ChildMethodInfo, namespace: &str, client: &syn::Path) -> syn::Result<TokenStream> {
    let Type::Path(tp) = &c.return_ty else {
        return Err(syn::Error::new_spanned(
            &c.return_ty,
            "#[plexus_macros::activation(client)]: a static #[child] must return its activation type by \
             name (`-> Inputs`) so the client can reach it through `InputsClient`",
        ));
    };
    let mut child_client = tp.path.clone();
    let Some(last) = child_client.segments.last_mut() else {
        return Err(syn::Error::new_spanned(&c.return_ty, "#[child]: empty return type path"));
    };
    // MACRO-13: a child named `…Activation` has its client named from the
    // stem (`InputsActivation` → `InputsClient`), as its own expansion names it.
    let child_name = last.ident.to_string();
    let stem = crate::codegen::contract::stem_of(&child_name).unwrap_or(&child_name);
    let ident = format_ident!("{}Client", stem, span = last.ident.span());
    last.ident = ident.clone();
    last.arguments = syn::PathArguments::None;
    if tp.qself.is_some() {
        return Err(syn::Error::new_spanned(
            &c.return_ty,
            "#[plexus_macros::activation(client)]: a static #[child]'s return type must be a plain path",
        ));
    }

    let fn_name = &c.fn_name;
    let segment = fn_name.to_string();
    let doc = if c.description.is_empty() {
        format!("The `{segment}` child — its methods go to `{namespace}.{segment}.<method>`.")
    } else {
        format!(
            "{}\n\nThe `{segment}` child — its methods go to `{namespace}.{segment}.<method>` \
             (or beneath wherever this client is mounted).",
            c.description
        )
    };
    Ok(quote! {
        #[doc = #doc]
        pub fn #fn_name(&self) -> #child_client<&T> {
            #child_client::at(
                &self.transport,
                #client::__rt::join_path(self.path(), #segment),
            )
        }
    })
}
