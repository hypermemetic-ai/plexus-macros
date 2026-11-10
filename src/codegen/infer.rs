//! KEEL-27 · what an unmarked fn is, read from how it is written.
//!
//! Under `#[activation(served = written)]` a fn of the block with no marker
//! is classified by its tokens — the macro emits different items per role,
//! so the classification is the macro's — and the compiler then *checks*
//! the one part tokens cannot see (that a mount's type is an activation),
//! never guesses it:
//!
//! | the fn, unmarked | is |
//! | -- | -- |
//! | not `pub` | a helper: not served, emitted as written |
//! | `pub`, not on `&self` (`new`, `&mut self`, `self`) | a helper: nothing to serve it on |
//! | `pub fn x(&self) -> A` | a mount (static), checked `A: ChildRouter` |
//! | `pub fn x(&self, name: &str) -> Option<A>` | a slot, checked `Option<A>`, `A: ChildRouter` |
//! | `pub async fn x(&self, name: &str) -> Result<Box<dyn I>, PeerError>` | a fallible mount (`#[peer]`) |
//! | any other `pub async fn` on `&self` | a method (a bare `#[method]`) |
//!
//! A fn with a marker is what the marker says; so is a sync sibling a
//! marked mount names (`#[child(list = names)]`: `names` stays a helper);
//! `plugin_children`, the hand-written hook the macro reads by name, stays a
//! helper.

use crate::parse::is_str_ref;
use proc_macro2::TokenStream;
use quote::{format_ident, quote, quote_spanned};
use syn::{spanned::Spanned, FnArg, ImplItemFn, ReturnType, Visibility};

/// What an unmarked fn is.
pub(crate) enum Inferred {
    /// Not served.
    Helper,
    /// A wire method, as a bare `#[method]`.
    Method,
    /// A mount, as a bare `#[child]` (static, or a slot by its `name: &str`).
    Mount,
    /// A fallible mount, as a bare `#[peer]`.
    FallibleMount,
}

/// What a mount returns, said wherever one is refused.
fn mount_returns(name: &syn::Ident) -> String {
    format!(
        "a mount returns an activation: `pub fn {name}(&self) -> A` (a static mount), \
         `pub fn {name}(&self, name: &str) -> Option<A>` (a slot), or \
         `pub async fn {name}(&self, name: &str) -> Result<Box<dyn I>, PeerError>` (a fallible \
         mount), `A` an activation (a `ChildRouter`); a fn that is not served is not `pub`"
    )
}

/// Classify an unmarked fn of a `served = written` block.
/// `siblings`: the fns a marked mount names as its `list` / `search`, whose
/// role is that marker's — a sync one stays a helper, as it is today.
pub(crate) fn classify(method: &ImplItemFn, siblings: &[syn::Ident]) -> syn::Result<Inferred> {
    let on_ref_self = matches!(
        method.sig.inputs.first(),
        Some(FnArg::Receiver(r)) if r.reference.is_some() && r.mutability.is_none()
    );
    if !matches!(method.vis, Visibility::Public(_)) || !on_ref_self || method.sig.ident == "plugin_children" {
        return Ok(Inferred::Helper);
    }
    let name = &method.sig.ident;
    let args: Vec<&syn::PatType> = method
        .sig
        .inputs
        .iter()
        .filter_map(|a| match a {
            FnArg::Typed(pt) => Some(pt),
            FnArg::Receiver(_) => None,
        })
        .collect();
    let named = args.len() == 1 && is_str_ref(&args[0].ty);

    if method.sig.asyncness.is_none() && siblings.contains(name) {
        return Ok(Inferred::Helper);
    }

    if method.sig.asyncness.is_some() {
        let fallible = match &method.sig.output {
            ReturnType::Type(_, ty) => is_fallible_mount(ty),
            ReturnType::Default => false,
        };
        if !fallible {
            return Ok(Inferred::Method);
        }
        if !named {
            return Err(syn::Error::new(
                name.span(),
                format!(
                    "`{name}` returns `Result<Box<dyn …>, PeerError>`, so it is served as a fallible \
                     mount, and a fallible mount takes a name: \
                     `pub async fn {name}(&self, name: &str) -> Result<Box<dyn I>, PeerError>`"
                ),
            ));
        }
        return Ok(Inferred::FallibleMount);
    }

    // A public sync fn on `&self` is a mount.
    if matches!(method.sig.output, ReturnType::Default) {
        return Err(syn::Error::new(
            name.span(),
            format!("`pub fn {name}` returns nothing, and a `pub fn` is served as a mount: {}", mount_returns(name)),
        ));
    }
    if !(args.is_empty() || named) {
        // The parameter that makes it no mount: the first that is not a
        // name, or the second.
        let at = args.iter().find(|a| !is_str_ref(&a.ty)).unwrap_or_else(|| &args[1]);
        return Err(syn::Error::new(
            at.span(),
            format!(
                "`pub fn {name}` is served as a mount, and a mount takes no argument or one \
                 `name: &str`: {}",
                mount_returns(name)
            ),
        ));
    }
    Ok(Inferred::Mount)
}

/// `Result<Box<dyn I>, PeerError>`, as written: a fallible mount.
fn is_fallible_mount(ty: &syn::Type) -> bool {
    if crate::codegen::peer::peer_shape(ty).is_err() {
        return false;
    }
    let syn::Type::Path(tp) = ty else { return false };
    let Some(seg) = tp.path.segments.last() else { return false };
    let syn::PathArguments::AngleBracketed(a) = &seg.arguments else { return false };
    matches!(
        a.args.iter().nth(1),
        Some(syn::GenericArgument::Type(syn::Type::Path(e)))
            if e.path.segments.last().is_some_and(|s| s.ident == "PeerError")
    )
}

/// The compiler's half: a hidden associated fn holding the bound a mount's
/// return type must meet, each refusal saying the fn and what a mount
/// returns (`#[diagnostic::on_unimplemented]` on a trait of the fn's own).
/// It is never called; it type-checks where the impl's generics and `Self`
/// are in scope.
pub(crate) fn mount_check(method: &ImplItemFn, crate_path: &syn::Path) -> TokenStream {
    let name = &method.sig.ident;
    let ReturnType::Type(_, ret) = &method.sig.output else { return quote! {} };
    let check = format_ident!("__plexus_mount_{}", name);
    let returns = mount_returns(name);
    let slot = method.sig.inputs.len() == 2;
    let body = if slot {
        let not_option = format!(
            "`{name}` takes `name: &str`, so it is served as a slot, and a slot returns \
             `Option<A>`, `A` an activation — not `{{Self}}`"
        );
        let not_mount =
            format!("`{name}` is served as a slot, and the `Option` it returns holds `{{Self}}`, which is not an activation");
        let call = quote_spanned! {ret.span()=> slot::<#ret>(); };
        quote! {
            #[diagnostic::on_unimplemented(message = #not_option, label = "not `Option` of an activation", note = #returns)]
            trait Slot { type Mount; }
            impl<A> Slot for ::core::option::Option<A> { type Mount = A; }
            #[diagnostic::on_unimplemented(message = #not_mount, label = "not an activation", note = #returns)]
            trait Mount {}
            impl<A: #crate_path::plexus::ChildRouter + 'static> Mount for A {}
            fn slot<S: Slot>() where S::Mount: Mount {}
            #call
        }
    } else {
        let not_mount = format!("`{name}` is served as a mount, and returns `{{Self}}`, which is not an activation");
        let call = quote_spanned! {ret.span()=> mount::<#ret>(); };
        quote! {
            #[diagnostic::on_unimplemented(message = #not_mount, label = "not an activation", note = #returns)]
            trait Mount {}
            impl<A: #crate_path::plexus::ChildRouter + 'static> Mount for A {}
            fn mount<M: Mount>() {}
            #call
        }
    };
    quote! {
        #[doc(hidden)]
        #[allow(dead_code, non_snake_case, clippy::all)]
        fn #check() { #body }
    }
}
