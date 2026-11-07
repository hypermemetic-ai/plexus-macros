//! NEST-7 · the primitive: an activation returns an activation.
//!
//! Two halves:
//!
//! - **the slot**: `#[plexus_macros::peer(list = …, search = …)] async fn
//!   x(&self, name: &str) -> Result<Box<dyn I>, PeerError>` — a typed peer
//!   when `I` is a contract's peer trait (`forge::ForgePeer`), untyped when
//!   it is `Peer`. Its role is `MethodRole::Peer { interface }` (the
//!   contract's interface, or none), routing calls it and hands the router
//!   the peer (a typed one checked against the slot's interface first:
//!   `ShapeMismatch` by interface hash), and every failure is a typed error
//!   item.
//! - **the contract**: a bodiless activation every method of which answers
//!   `Result<T, E>` (one item) is a typed slot's interface: beside its trait
//!   `Forge`, the macro emits `trait ForgePeer: Forge + Peer` (blanket), its
//!   `Contract` (the interface and the `Stamp`), and `Forge` + `Peer` for
//!   `ForgeClient<PeerTransport<T>>` — the remote peer, checked against the
//!   stamp when it becomes one (`ForgeClient::peer(transport)`) and again
//!   whenever an item's `plexus_hash` moves. Every method's `E` must be
//!   `From<PeerError>`: a call across the network can fail in transit.

use crate::parse::{Cardinality, MethodInfo};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};

/// A `#[peer]` method's return: `Result<Box<dyn Path>, _>`.
#[derive(Debug, Clone)]
pub(crate) enum PeerShape {
    /// `Box<dyn Peer>`: admitted whole.
    Untyped,
    /// `Box<dyn some::ContractPeer>`.
    Typed(syn::Path),
}

/// The shape of `Result<Box<dyn X>, E>`, or why it is not one.
pub(crate) fn peer_shape(ty: &syn::Type) -> Result<PeerShape, &'static str> {
    const WANT: &str = "a #[plexus_macros::peer] method returns `Result<Box<dyn I>, PeerError>` (typed: `I` a contract's peer trait) or `Result<Box<dyn Peer>, PeerError>` (untyped)";
    let first_arg = |ty: &syn::Type, name: &str| -> Option<syn::Type> {
        let syn::Type::Path(tp) = ty else { return None };
        let seg = tp.path.segments.last()?;
        if seg.ident != name {
            return None;
        }
        let syn::PathArguments::AngleBracketed(a) = &seg.arguments else { return None };
        match a.args.first()? {
            syn::GenericArgument::Type(t) => Some(t.clone()),
            _ => None,
        }
    };
    let boxed = first_arg(ty, "Result").ok_or(WANT)?;
    let object = first_arg(&boxed, "Box").ok_or(WANT)?;
    let syn::Type::TraitObject(to) = object else { return Err(WANT) };
    let path = to
        .bounds
        .iter()
        .find_map(|b| match b {
            syn::TypeParamBound::Trait(t) => Some(t.path.clone()),
            _ => None,
        })
        .ok_or(WANT)?;
    if path.segments.last().is_some_and(|s| s.ident == "Peer") {
        Ok(PeerShape::Untyped)
    } else {
        Ok(PeerShape::Typed(path))
    }
}

/// The slot's interface, as an expression of type `Option<Interface>`.
pub(crate) fn interface_expr(shape: &PeerShape, crate_path: &syn::Path) -> TokenStream {
    match shape {
        PeerShape::Untyped => quote! { ::std::option::Option::None },
        PeerShape::Typed(path) => quote! {
            ::std::option::Option::Some(<dyn #path as #crate_path::plexus::peer::Contract>::interface())
        },
    }
}

/// A contract is peerable when every method answers one `Result<T, E>` and
/// takes no input stream.
pub(crate) fn peerable(methods: &[MethodInfo]) -> bool {
    !methods.is_empty()
        && methods.iter().all(|m| matches!(m.cardinality, Cardinality::One) && m.err_ty.is_some() && m.input.is_none())
}

/// The contract's peer half: `trait {Trait}Peer`, its `Contract`, and the
/// client over a `PeerTransport` as a `{Trait}` and a `Peer`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn contract_peer_items(
    trait_ident: &syn::Ident,
    struct_name: &syn::Ident,
    namespace: &str,
    description: &str,
    version_expr: &TokenStream,
    methods: &[MethodInfo],
    future: &TokenStream,
    client_ident: &syn::Ident,
    client_path: &syn::Path,
    crate_path: &syn::Path,
    deps: &syn::Ident,
) -> TokenStream {
    let peer_ident = format_ident!("{}Peer", trait_ident);
    let enum_name = format_ident!("{}Method", struct_name);
    let peer = quote! { #crate_path::plexus::peer };
    let doc = format!(
        "NEST-7: a `{trait_ident}` that is a peer — a local implementation served (`x.server()`) or \
         the remote one (`{client_ident}::peer(transport)`); what a typed slot returns \
         (`Result<Box<dyn {peer_ident}>, PeerError>`). Implemented for every `{trait_ident} + Peer`."
    );
    let impls = methods.iter().map(|m| {
        let fn_ident = &m.fn_name;
        let name = m.method_name.as_str();
        let params: Vec<TokenStream> = m.params.iter().map(|p| { let (n, t) = (&p.name, &p.ty); quote! { #n: #t } }).collect();
        let names: Vec<&syn::Ident> = m.params.iter().map(|p| &p.name).collect();
        let ok = &m.ok_ty;
        let err = m.err_ty.as_ref().expect("peerable: every method is fallible");
        quote! {
            #[allow(clippy::too_many_arguments)]
            fn #fn_ident(&self #(, #params)*) -> #future<'static, ::core::result::Result<#ok, #err>> {
                let __t = ::core::clone::Clone::clone(self.transport());
                let __path = ::std::string::ToString::to_string(self.path());
                ::std::boxed::Box::pin(async move {
                    let __stamp = <dyn #peer_ident as #peer::Contract>::stamp();
                    __t.guard(&__path, __stamp, #name).await.map_err(<#err as ::core::convert::From<#peer::PeerError>>::from)?;
                    let __client = #client_ident::at(::core::clone::Clone::clone(&__t), ::core::clone::Clone::clone(&__path));
                    let __r = __client.#fn_ident(#(#names),*).await;
                    __t.after(&__path, __stamp, #name).await.map_err(<#err as ::core::convert::From<#peer::PeerError>>::from)?;
                    match __r {
                        ::core::result::Result::Ok(__inner) => __inner,
                        ::core::result::Result::Err(__e) => ::core::result::Result::Err(
                            <#err as ::core::convert::From<#peer::PeerError>>::from(#peer::PeerError::from_client(&__e)),
                        ),
                    }
                })
            }
        }
    });
    let bound = quote! { #client_path::Transport + ::core::clone::Clone + ::core::marker::Send + ::core::marker::Sync + 'static };
    quote! {
        #[doc = #doc]
        pub trait #peer_ident: #trait_ident + #peer::Peer {}
        impl<__X: #trait_ident + #peer::Peer + ?::core::marker::Sized> #peer_ident for __X {}

        impl #peer::Contract for dyn #peer_ident {
            fn interface() -> #peer::Interface {
                static __I: ::std::sync::OnceLock<#peer::Interface> = ::std::sync::OnceLock::new();
                ::core::clone::Clone::clone(__I.get_or_init(|| {
                    let __schema = #crate_path::plexus::PluginSchema::leaf(#namespace, #version_expr, #description, #enum_name::method_schemas());
                    #peer::Interface::named(#namespace, &__schema)
                }))
            }

            fn stamp() -> &'static #peer::Stamp {
                static __S: ::std::sync::OnceLock<#peer::Stamp> = ::std::sync::OnceLock::new();
                __S.get_or_init(|| #peer::Stamp::of(#version_expr, &<dyn #peer_ident as #peer::Contract>::interface()))
            }
        }

        impl<__T> #client_ident<__T> {
            /// NEST-7: what this client was generated from — the backend
            /// crate's version, the interface hash and each method's hash.
            pub fn stamp() -> &'static #peer::Stamp {
                <dyn #peer_ident as #peer::Contract>::stamp()
            }
        }

        impl<__T: #bound> #client_ident<#peer::PeerTransport<__T>> {
            /// NEST-7: become a peer over `transport` — the backend's
            /// interface checked against [`Self::stamp`]: equal, or every
            /// method the same (`drift`, said once), else `ShapeMismatch`.
            pub async fn peer(transport: __T) -> ::core::result::Result<Self, #peer::PeerError> {
                Self::peer_at(transport, #namespace).await
            }

            /// [`Self::peer`] for the contract served at `path`.
            pub async fn peer_at(transport: __T, path: impl ::core::convert::Into<::std::string::String>) -> ::core::result::Result<Self, #peer::PeerError> {
                let __path: ::std::string::String = path.into();
                let __t = #peer::PeerTransport::new(transport);
                __t.admit(&__path, Self::stamp()).await?;
                ::core::result::Result::Ok(Self::at(__t, __path))
            }
        }

        #[#deps::async_trait::async_trait]
        impl<__T: #bound> #peer::Peer for #client_ident<#peer::PeerTransport<__T>> {
            fn interface(&self) -> #peer::Interface {
                self.transport().interface().unwrap_or_else(|| <dyn #peer_ident as #peer::Contract>::interface())
            }

            async fn call(
                &self,
                method: &str,
                params: #deps::serde_json::Value,
                _auth: ::core::option::Option<&#crate_path::plexus::AuthContext>,
            ) -> ::core::result::Result<#crate_path::plexus::PlexusStream, #crate_path::plexus::PlexusError> {
                let __known = <Self as #peer::Peer>::interface(self);
                self.transport().call_peer(self.path(), method, params, &__known).await
            }

            async fn peer(&self, name: &str) -> ::core::result::Result<::std::boxed::Box<dyn #peer::Peer>, #peer::PeerError> {
                ::core::result::Result::Err(#peer::PeerError::NotFound { name: ::std::string::ToString::to_string(name) })
            }
        }

        impl<__T: #bound> #trait_ident for #client_ident<#peer::PeerTransport<__T>> {
            #(#impls)*
        }
    }
}
