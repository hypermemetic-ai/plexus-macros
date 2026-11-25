//! Classify method arguments once for the IR and for native dispatch.

use plexus_ir::{Caller, Field, Method};
use proc_macro2::{Ident, TokenStream};
use quote::{format_ident, quote};
use syn::{FnArg, Pat, Signature, Type};

use super::{Res, attr, err, extras, is_named, is_shared_ref, kebab, last_segment, ty, type_args};

#[derive(Default)]
pub(super) struct MethodArgs {
    /// The ordinary parameters read from the open frame, in IR order.
    pub(super) wire_names: Vec<Ident>,
    pub(super) wire_types: Vec<Type>,
    /// Every argument passed to the Rust method, including injected values, in source order.
    pub(super) invocation: Vec<TokenStream>,
    pub(super) takes_context: bool,
    pub(super) injections: Vec<(Ident, Injection)>,
    pub(super) input_type: Option<Type>,
}

pub(super) enum Injection {
    Peer(Type),
    User(Type),
}

impl Injection {
    pub(super) fn resolve(&self) -> TokenStream {
        match self {
            Self::Peer(interface) => quote!(__plexus_call.peer::<#interface>()),
            Self::User(ty) => quote!(<#ty as ::plexus::FromCaller<_>>::from_caller(
                __plexus_get(&*__plexus_a), __plexus_required_caller
            )),
        }
    }
}

fn user_type(ty: &Type) -> bool {
    let Type::Path(ty) = ty else { return false };
    ty.qself.is_none()
        && (ty.path.is_ident("User")
            || (ty.path.segments.len() == 2
                && ty.path.segments[0].ident == "plexus"
                && ty.path.segments[1].ident == "User"
                && ty.path.segments.iter().all(|s| s.arguments.is_none())))
}

/// The typed interface requested by an activation-level peer injection.
pub(super) fn peer_interface(ty: &Type) -> Res<Option<&Type>> {
    let Some(seg) = last_segment(ty).filter(|seg| seg.ident == "Peer") else {
        return Ok(None);
    };
    if let Type::Path(_) = ty
        && let syn::PathArguments::AngleBracketed(args) = &seg.arguments
        && args.args.len() == 1
        && let Some(syn::GenericArgument::Type(interface)) = args.args.first()
    {
        return Ok(Some(interface));
    }
    err(ty, "a peer is an owned Peer<Activation>")
}

impl MethodArgs {
    pub(super) fn read(
        signature: &Signature,
        method: &mut Method,
        context: Option<&Ident>,
        describe: bool,
    ) -> Res<Self> {
        let mut args = Self::default();
        let caller_type = format_ident!("Caller");
        for arg in signature.inputs.iter().skip(1) {
            let FnArg::Typed(t) = arg else { continue };
            let Pat::Ident(p) = &*t.pat else {
                return err(&t.pat, "a parameter is a plain name");
            };
            let seg = last_segment(&t.ty);
            let inner_type = seg.and_then(|s| type_args(&s.arguments).first().copied());
            if let Some(selector) = attr(&t.attrs, "peer") {
                return err(
                    selector,
                    "declare Peer<T> without a selector and bind its URL and key in the application with PeerHost::inject_peer",
                );
            }
            let injection = if let Some(inner) = peer_interface(&t.ty)? {
                Some(Injection::Peer(inner.clone()))
            } else if user_type(&t.ty) {
                if method.caller.is_some() {
                    return err(&t.ty, "a method takes at most one caller");
                }
                method.caller = Some(Caller::Required);
                Some(Injection::User((*t.ty).clone()))
            } else {
                None
            };
            if let Some(injection) = injection {
                let local = format_ident!("__plexus_injection_{}", args.injections.len());
                args.invocation.push(quote!(#local));
                args.injections.push((local, injection));
                continue;
            }
            let caller = if is_named(&t.ty, &caller_type) {
                Some((Caller::Required, &*t.ty))
            } else if seg.is_some_and(|s| s.ident == "Option") {
                inner_type
                    .filter(|t| is_named(t, &caller_type))
                    .map(|t| (Caller::Optional, t))
            } else {
                None
            };
            if let Some((caller, inner)) = caller {
                if !describe
                    && (!is_shared_ref(inner, &caller_type)
                        || (caller == Caller::Optional && !matches!(&*t.ty, Type::Path(_))))
                {
                    return err(&t.ty, "an injected caller is &Caller or Option<&Caller>");
                }
                if method.caller.is_some() {
                    return err(&t.ty, "a method takes at most one caller");
                }
                args.invocation.push(match caller {
                    Caller::Required => quote!(__plexus_required_caller),
                    Caller::Optional => quote!(__plexus_call.caller()),
                });
                method.caller = Some(caller);
                continue;
            }
            if let Some(name) = context.filter(|name| is_named(&t.ty, name)) {
                if args.takes_context || !is_shared_ref(&t.ty, name) {
                    return err(
                        &t.ty,
                        "a method takes at most one shared reference to the activation context",
                    );
                }
                args.takes_context = true;
                args.invocation.push(quote!(&__plexus_context));
                continue;
            }
            match seg.map(|s| s.ident.to_string()).as_deref() {
                Some("Input") => {
                    if method.input.is_some() {
                        return err(&t.ty, "a method takes at most one Input<T>");
                    }
                    let Some(inner) = inner_type.filter(|_| {
                        matches!(&*t.ty, Type::Path(_))
                            && seg.is_some_and(|s| type_args(&s.arguments).len() == 1)
                    }) else {
                        return err(&t.ty, "an input stream is an owned Input<T>");
                    };
                    args.input_type = Some(inner.clone());
                    method.input = Some(ty(inner)?);
                    args.invocation.push(quote!(__plexus_input));
                }
                Some("Progress") if describe => method.progress = inner_type.map(ty).transpose()?,
                Some("Progress") => {
                    return err(
                        &t.ty,
                        "progress is only described: use #[activation(describe)]",
                    );
                }
                _ => {
                    let mut field = Field {
                        name: kebab(&p.ident),
                        ty: Some(ty(&t.ty)?),
                        ..Field::default()
                    };
                    extras(&t.attrs, &mut field)?;
                    if attr(&t.attrs, "target").is_some() {
                        method.target = Some(field.name.clone());
                    }
                    method.params.push(field);
                    args.wire_names.push(p.ident.clone());
                    args.wire_types.push((*t.ty).clone());
                    let ident = &p.ident;
                    args.invocation.push(quote!(#ident));
                }
            }
        }
        Ok(args)
    }
}
