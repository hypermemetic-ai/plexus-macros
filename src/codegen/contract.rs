//! MACRO-13 · `#[activation]` needs nothing.
//!
//! On `impl FooActivation` the stem `Foo` names the namespace (`foo`), the
//! trait (`trait Foo`) and the client (`FooClient<T>`); `trait = "Bar"` names
//! them whatever the type is called. Every `pub async fn` on `&self`
//! returning one of RETURNS-8's signature forms is a wire method (`methods =
//! marked` restores the `#[method]`-only rule).
//!
//! RETURNS-36: the trait carries one event type `E` across the activation,
//! each method `-> BoxStream<'static, E>` — the shape of an activation whose
//! wire methods are all `impl Stream<Item = E>` of one `E`, which every
//! activation written before 0.7 has. It is emitted for exactly those (the
//! owner's `always`, 2026-10-16, for every activation that has the shape);
//! an activation that gives its methods their own return types (the owner,
//! 2026-11-02: "each method declared under its own return type") has its
//! client and its aggregate `{Ns}Event`, and no trait. A contract (a bodiless
//! activation) is its trait, so it must have the shape. A crate that declares a `server` feature gets its server half gated
//! on it. A bodiless activation is a contract: the macro emits
//! `FooActivation<D: Foo>`, delegating to `D`, and `Foo::server(self)`.
//!
//! A type not named `…Activation` and without `trait = "…"` cannot be named,
//! and is refused — except an activation written before MACRO-13, which says
//! its `namespace`: it expands as it did (no trait, the client on the
//! `client` flag, only `#[method]`s on the wire), with a deprecation warning
//! asking for the rename, so that every repository that takes the macro by
//! path keeps building until its own rename lands.

use crate::parse::{classify_return, Cardinality, HubMethodsAttrs, MethodInfo};
use proc_macro2::{Span, TokenStream};
use proc_macro_crate::{crate_name, FoundCrate};
use quote::{format_ident, quote};
use syn::{
    parse::{Parse, ParseStream},
    FnArg, ImplItem, ImplItemFn, Signature, Token, Type, Visibility,
};

const SUFFIX: &str = "Activation";

/// What the name rule makes of an activation.
pub(crate) struct Names {
    /// The rule applies: the type is named `…Activation`, or the attribute
    /// names the trait. `false` only for an activation written before
    /// MACRO-13 (a type the rule cannot name, with its `namespace` given).
    pub new_form: bool,
    /// The trait's name — the stem, or `trait = "…"`. `None` exactly when
    /// `new_form` is false.
    pub trait_ident: Option<syn::Ident>,
    pub namespace: String,
    pub client_ident: syn::Ident,
}

fn refusal(struct_name: &syn::Ident) -> syn::Error {
    syn::Error::new(
        struct_name.span(),
        format!(
            "`{struct_name}` does not end in `Activation`, so #[plexus_macros::activation] cannot \
             name the trait, the client or the namespace. Name the type `{struct_name}Activation`, \
             or write `trait = \"…\"` in the attribute."
        ),
    )
}

/// The stem the name rule takes from a type name: `FooActivation` → `Foo`.
pub(crate) fn stem_of(type_name: &str) -> Option<&str> {
    type_name.strip_suffix(SUFFIX).filter(|s| !s.is_empty())
}

impl Names {
    pub fn resolve(args: &HubMethodsAttrs, struct_name: &syn::Ident) -> syn::Result<Self> {
        let type_name = struct_name.to_string();
        let type_stem = stem_of(&type_name).map(|s| syn::Ident::new(s, struct_name.span()));
        let explicit = args
            .trait_attr
            .as_ref()
            .map(|lit| syn::Ident::new(&lit.value(), lit.span()));
        let stem = explicit.or(type_stem);

        let Some(stem) = stem else {
            // Before MACRO-13 every activation said its namespace; one that
            // still does keeps its old expansion, with a deprecation warning
            // (`legacy_name_warning`). One that does not cannot be named.
            return match &args.namespace {
                Some(ns) => Ok(Names {
                    new_form: false,
                    trait_ident: None,
                    namespace: ns.clone(),
                    client_ident: format_ident!("{}Client", struct_name),
                }),
                None => Err(refusal(struct_name)),
            };
        };

        Ok(Names {
            new_form: true,
            namespace: args.namespace.clone().unwrap_or_else(|| snake_case(&stem.to_string())),
            client_ident: format_ident!("{}Client", stem),
            trait_ident: Some(stem),
        })
    }
}

/// An activation written before MACRO-13 on a type the name rule cannot
/// name: it still builds, as it did, and says what to rename.
pub(crate) fn legacy_name_warning(struct_name: &syn::Ident) -> TokenStream {
    let span = struct_name.span();
    let const_name = format_ident!("_PLEXUS_MACROS_DEPRECATED_UNNAMED_ACTIVATION_{}", struct_name, span = span);
    let touch = format_ident!("_plexus_macros_touch_unnamed_activation_{}", struct_name, span = span);
    let note = format!(
        "plexus-macros (MACRO-13): `{struct_name}` does not end in `Activation`, so no trait is \
         emitted for it and its client is `{struct_name}Client`. Rename it `{struct_name}Activation` \
         (or write `trait = \"…\"`); then `namespace`, `client`, `crate_path`, `server_cfg` and \
         `#[method]` may all go. A later release refuses it."
    );
    quote::quote_spanned! { span =>
        #[doc(hidden)]
        #[allow(non_upper_case_globals, non_snake_case, dead_code)]
        #[deprecated(since = "0.6.0", note = #note)]
        const #const_name: () = ();

        #[doc(hidden)]
        #[allow(non_snake_case, dead_code)]
        fn #touch() {
            let _ = #const_name;
        }
    }
}

/// `TicketsApi` → `tickets_api`, `HTTPServer` → `http_server`.
pub(crate) fn snake_case(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if c.is_uppercase() {
            let prev = i.checked_sub(1).map(|j| chars[j]);
            let next = chars.get(i + 1).copied();
            let boundary = match prev {
                None => false,
                Some(p) if p == '_' => false,
                Some(p) if p.is_lowercase() || p.is_ascii_digit() => true,
                Some(p) if p.is_uppercase() => next.is_some_and(|n| n.is_lowercase()),
                _ => false,
            };
            if boundary {
                out.push('_');
            }
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// `pub async fn f(&self, …) -> T;` — a method with no body. syn hands it to
/// us as `ImplItem::Verbatim` (rustc refuses it only after expansion).
struct BodilessFn {
    attrs: Vec<syn::Attribute>,
    vis: Visibility,
    sig: Signature,
}

impl Parse for BodilessFn {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let attrs = input.call(syn::Attribute::parse_outer)?;
        let vis: Visibility = input.parse()?;
        let sig: Signature = input.parse()?;
        input.parse::<Token![;]>()?;
        Ok(BodilessFn { attrs, vis, sig })
    }
}

/// Turn every bodiless method of the impl into an `ImplItem::Fn` with a
/// placeholder body; return their names, in source order. The placeholder is
/// replaced by the contract's delegation, or the activation is refused.
pub(crate) fn take_bodiless(input_impl: &mut syn::ItemImpl) -> Vec<syn::Ident> {
    let mut names = Vec::new();
    for item in &mut input_impl.items {
        if let ImplItem::Verbatim(tokens) = item {
            if let Ok(f) = syn::parse2::<BodilessFn>(tokens.clone()) {
                names.push(f.sig.ident.clone());
                *item = ImplItem::Fn(ImplItemFn {
                    attrs: f.attrs,
                    vis: f.vis,
                    defaultness: None,
                    sig: f.sig,
                    block: syn::parse_quote!({ ::core::unreachable!() }),
                });
            }
        }
    }
    names
}

/// Under the name rule (and not `methods = marked`): is this unmarked method
/// on the wire? `pub async fn` on `&self` returning one of the six signature
/// forms (RETURNS-8) is; one returning anything else is refused; any other
/// shape is a plain method.
pub(crate) fn is_wire_shape(method: &ImplItemFn) -> syn::Result<bool> {
    let public = matches!(method.vis, Visibility::Public(_));
    let on_ref_self = matches!(
        method.sig.inputs.first(),
        Some(FnArg::Receiver(r)) if r.reference.is_some() && r.mutability.is_none()
    );
    if !(public && method.sig.asyncness.is_some() && on_ref_self) {
        return Ok(false);
    }
    match classify_return(&method.sig) {
        Ok(_) => Ok(true),
        Err(form) => {
            let name = &method.sig.ident;
            let returns = match &method.sig.output {
                syn::ReturnType::Type(_, ty) => quote!(#ty).to_string(),
                syn::ReturnType::Default => "()".to_string(),
            };
            let mut e = syn::Error::new(
                name.span(),
                format!(
                    "`pub async fn {name}(&self, …)` returns `{returns}`, which is not a signature \
                     form, so it cannot go on the wire — and every `pub async fn` on `&self` of an \
                     activation is a wire method. Make `{name}` non-`pub`, or write `methods = \
                     marked` on the activation (then only #[plexus_macros::method]s are on the wire)."
                ),
            );
            e.combine(form);
            Err(e)
        }
    }
}

/// Does the consuming crate's manifest declare a `server` feature?
pub(crate) fn manifest_declares_server_feature() -> bool {
    let Ok(dir) = std::env::var("CARGO_MANIFEST_DIR") else { return false };
    let Ok(text) = std::fs::read_to_string(std::path::Path::new(&dir).join("Cargo.toml")) else {
        return false;
    };
    let mut table = String::new();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.starts_with('[') {
            table = line.trim_matches(|c| c == '[' || c == ']').trim().to_string();
            continue;
        }
        let Some((key, _)) = line.split_once('=') else { continue };
        let key = key.trim().trim_matches('"');
        if (table == "features" && key == "server") || (table.is_empty() && key == "features.server") {
            return true;
        }
    }
    false
}

/// The `BoxStream<'static, E>` a trait method returns: plexus-client's when
/// the crate has it, else futures' (directly or through the umbrella).
pub(crate) fn box_stream_path(client_path: Option<&syn::Path>, span: Span) -> syn::Result<TokenStream> {
    if let Some(client) = client_path {
        return Ok(quote! { #client::BoxStream });
    }
    if let Ok(found) = crate_name("futures") {
        let ident = match found {
            FoundCrate::Itself => format_ident!("futures"),
            FoundCrate::Name(n) => syn::Ident::new(&n, span),
        };
        return Ok(quote! { ::#ident::stream::BoxStream });
    }
    if let Some(umbrella) = crate::codegen::umbrella_crate(span) {
        return Ok(quote! { ::#umbrella::__deps::futures::stream::BoxStream });
    }
    Err(syn::Error::new(
        span,
        "#[plexus_macros::activation]: the activation's trait returns boxed streams, and this \
         crate depends on neither `plexus-client` nor `futures` to name one. Add `plexus-client` \
         (which also gives you the client) or `futures`.",
    ))
}

/// What the server injects into a method (and a caller never passes), if
/// anything: the activation cannot call such a method on itself, so it
/// cannot implement its own trait.
pub(crate) fn injected(m: &MethodInfo) -> Option<&'static str> {
    if !m.activation_params.is_empty() {
        Some("an `#[activation_param]`")
    } else if !m.auth_resolvers.is_empty() {
        Some("a `#[from_auth]` parameter")
    } else if m.requires_auth {
        Some("an `auth` parameter")
    } else if m.takes_reach {
        Some("a `reach` parameter")
    } else if !matches!(m.bidirectional, crate::parse::BidirType::None) {
        Some("a bidirectional `ctx` parameter")
    } else {
        None
    }
}

/// Does a method take what the server injects? An activation with such a
/// method does not implement its own trait — it has no caller to take those
/// from. True when no wire method does.
pub(crate) fn self_callable(methods: &[MethodInfo]) -> bool {
    methods.iter().all(|m| injected(m).is_none())
}

/// A method's event type on the trait: `E` of `impl Stream<Item = E>`, a
/// plain stream (no `Result`, no `Progress`).
fn plain_stream_item(m: &MethodInfo) -> Option<&Type> {
    (m.cardinality == Cardinality::Many && m.err_ty.is_none() && m.progress_ty.is_none()).then_some(&m.ok_ty)
}

/// RETURNS-36: can the activation's methods be trait `trait_ident` — every
/// one `impl Stream<Item = E>` of one `E`? `Err` says why not, on the method
/// that breaks it.
pub(crate) fn trait_shape(methods: &[MethodInfo], trait_ident: &syn::Ident) -> syn::Result<()> {
    let mut first_event: Option<(String, &syn::Ident)> = None;
    for m in methods {
        let Some(event) = plain_stream_item(m) else {
            return Err(syn::Error::new(
                m.fn_name.span(),
                format!(
                    "`{}` does not return `impl Stream<Item = E>`, so trait `{trait_ident}` has no \
                     event type for it: the trait's methods yield one event type across the \
                     activation. Give it a stream return type.",
                    m.fn_name
                ),
            ));
        };
        let event_str = quote!(#event).to_string();
        match &first_event {
            None => first_event = Some((event_str, &m.fn_name)),
            Some((first, first_fn)) if *first != event_str => {
                return Err(syn::Error::new(
                    m.fn_name.span(),
                    format!(
                        "`{}` yields `{event_str}`, but `{first_fn}` yields `{first}`: the methods of \
                         trait `{trait_ident}` yield one event type across the activation.",
                        m.fn_name
                    ),
                ));
            }
            Some(_) => {}
        }
    }
    Ok(())
}

fn sig_params(m: &MethodInfo) -> (Vec<TokenStream>, Vec<&syn::Ident>) {
    let params = m
        .params
        .iter()
        .map(|p| {
            let (name, ty) = (&p.name, &p.ty);
            quote! { #name: #ty }
        })
        .collect();
    let names = m.params.iter().map(|p| &p.name).collect();
    (params, names)
}

fn event_ty(m: &MethodInfo) -> &Type {
    plain_stream_item(m).expect("checked by trait_shape")
}

/// `pub trait Foo: Send + Sync + 'static { fn m(&self, …) -> BoxStream<'static, E>; … }`,
/// its dyn-compatibility assertion, and — for a contract — `server()`.
pub(crate) fn trait_def(
    trait_ident: &syn::Ident,
    struct_name: &syn::Ident,
    description: &str,
    methods: &[MethodInfo],
    box_stream: &TokenStream,
    contract: bool,
    server_gate: Option<&syn::Meta>,
) -> TokenStream {
    let items = methods.iter().map(|m| {
        let fn_ident = &m.fn_name;
        let (params, _) = sig_params(m);
        let ev = event_ty(m);
        let doc = &m.description;
        let doc = if doc.is_empty() { quote! {} } else { quote! { #[doc = #doc] } };
        quote! {
            #doc
            #[allow(clippy::too_many_arguments)]
            fn #fn_ident(&self #(, #params)*) -> #box_stream<'static, #ev>;
        }
    });
    let server = if contract {
        let gate = server_gate.map(|m| quote! { #[cfg(#m)] });
        quote! {
            /// Serve this implementation on a hub: `hub.register(x.server())`.
            #gate
            fn server(self) -> #struct_name<Self>
            where
                Self: ::core::marker::Sized,
            {
                #struct_name(::std::sync::Arc::new(self))
            }
        }
    } else {
        quote! {}
    };
    let doc = if description.is_empty() {
        quote! {}
    } else {
        quote! { #[doc = #description] }
    };
    let assert_fn = format_ident!("__plexus_assert_{}_is_dyn_compatible", snake_case(&trait_ident.to_string()));
    quote! {
        #doc
        ///
        /// Emitted by `#[plexus_macros::activation]` (MACRO-13) from the
        /// activation's wire methods: implemented by the activation and by its
        /// client, so anything that implements it can be served and anything
        /// served can be called through it.
        pub trait #trait_ident: ::core::marker::Send + ::core::marker::Sync + 'static {
            #(#items)*
            #server
        }

        // MACRO-13: the trait is dyn-compatible by construction (boxed
        // streams, no generics, no `async fn`); this fails to compile the day
        // it is not.
        #[doc(hidden)]
        #[allow(dead_code, non_snake_case)]
        fn #assert_fn(_: &dyn #trait_ident) {}
    }
}

/// Server half, server form: `impl Foo for FooActivation`, each method the
/// inherent method's stream. The trait's stream is `'static` and the inherent
/// method's future borrows `self`, so the activation is cloned into the
/// stream (`Plexus::register` already asks every activation to be `Clone`).
pub(crate) fn activation_trait_impl(
    trait_ident: &syn::Ident,
    struct_name: &syn::Ident,
    self_ty: &Type,
    impl_generics: &syn::ImplGenerics,
    where_clause: Option<&syn::WhereClause>,
    methods: &[MethodInfo],
    box_stream: &TokenStream,
    deps: &syn::Ident,
) -> TokenStream {
    let items = methods.iter().map(|m| {
        let fn_ident = &m.fn_name;
        let (params, names) = sig_params(m);
        let ev = event_ty(m);
        quote! {
            #[allow(clippy::too_many_arguments)]
            fn #fn_ident(&self #(, #params)*) -> #box_stream<'static, #ev> {
                // The activation is cloned into the `'static` stream; a
                // type that is not `Clone` is told so, by name.
                #[diagnostic::on_unimplemented(
                    message = "`{Self}` must implement `Clone` to implement its activation trait",
                    label = "this activation is not `Clone`",
                    note = "#[plexus_macros::activation] (MACRO-13) implements the activation's trait by cloning the activation into each method's stream, as `register` does; derive or implement `Clone` for `{Self}` (wrap its state in `Arc`)"
                )]
                trait __PlexusActivationClone: ::core::marker::Sized {
                    fn __plexus_clone(&self) -> Self;
                }
                impl<__T: ::core::clone::Clone> __PlexusActivationClone for __T {
                    fn __plexus_clone(&self) -> Self {
                        ::core::clone::Clone::clone(self)
                    }
                }
                let __this = __PlexusActivationClone::__plexus_clone(self);
                ::std::boxed::Box::pin(#deps::futures::StreamExt::flatten(
                    #deps::futures::stream::once(async move {
                        #struct_name::#fn_ident(&__this #(, #names)*).await
                    }),
                ))
            }
        }
    });
    quote! {
        impl #impl_generics #trait_ident for #self_ty #where_clause {
            #(#items)*
        }
    }
}

/// The generic parameter of a contract's emitted struct.
pub(crate) fn contract_param() -> syn::Ident {
    format_ident!("__D")
}

/// Contract form: give each bodiless method its delegation, and make the
/// impl generic over the implementor.
pub(crate) fn rewrite_contract_impl(
    input_impl: &mut syn::ItemImpl,
    struct_name: &syn::Ident,
    trait_ident: &syn::Ident,
    methods: &[MethodInfo],
) -> syn::Result<()> {
    if !input_impl.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &input_impl.generics,
            "a bodiless activation is a contract, whose struct the macro emits generic over its \
             implementor; the impl takes no generics of its own",
        ));
    }
    match &*input_impl.self_ty {
        Type::Path(tp) if tp.qself.is_none() && tp.path.get_ident().is_some() => {}
        other => {
            return Err(syn::Error::new_spanned(
                other,
                "a bodiless activation is a contract: write it on the bare type name \
                 (`impl FooActivation`), whose struct the macro emits",
            ))
        }
    }
    let d = contract_param();
    input_impl.generics = syn::parse_quote!(<#d: #trait_ident>);
    input_impl.self_ty = Box::new(syn::parse_quote!(#struct_name<#d>));
    for item in &mut input_impl.items {
        if let ImplItem::Fn(f) = item {
            if let Some(m) = methods.iter().find(|m| m.fn_name == f.sig.ident) {
                let fn_ident = &m.fn_name;
                let (_, names) = sig_params(m);
                f.block = syn::parse_quote!({
                    <#d as #trait_ident>::#fn_ident(&*self.0 #(, #names)*)
                });
            }
        }
    }
    Ok(())
}

/// Contract form, server half: the struct, its constructors, `Clone` (what
/// `register` asks for, without asking it of the implementor), and
/// `impl<D: Foo> Foo for FooActivation<D>`.
pub(crate) fn contract_items(
    trait_ident: &syn::Ident,
    struct_name: &syn::Ident,
    methods: &[MethodInfo],
    box_stream: &TokenStream,
) -> TokenStream {
    let d = contract_param();
    let doc = format!(
        "The `{trait_ident}` contract served: an activation that delegates every wire method to \
         its implementor. Emitted by `#[plexus_macros::activation]` (MACRO-13) for a bodiless \
         activation; make one with `{trait_ident}::server(x)` or `{struct_name}::new(x)`."
    );
    let items = methods.iter().map(|m| {
        let fn_ident = &m.fn_name;
        let (params, names) = sig_params(m);
        let ev = event_ty(m);
        quote! {
            #[allow(clippy::too_many_arguments)]
            fn #fn_ident(&self #(, #params)*) -> #box_stream<'static, #ev> {
                <#d as #trait_ident>::#fn_ident(&*self.0 #(, #names)*)
            }
        }
    });
    quote! {
        #[doc = #doc]
        pub struct #struct_name<#d: #trait_ident>(::std::sync::Arc<#d>);

        impl<#d: #trait_ident> #struct_name<#d> {
            /// Serve `inner`.
            pub fn new(inner: #d) -> Self {
                Self(::std::sync::Arc::new(inner))
            }

            /// Serve an implementation already shared.
            pub fn from_arc(inner: ::std::sync::Arc<#d>) -> Self {
                Self(inner)
            }

            /// The implementation this activation serves.
            pub fn inner(&self) -> &#d {
                &self.0
            }
        }

        impl<#d: #trait_ident> ::core::clone::Clone for #struct_name<#d> {
            fn clone(&self) -> Self {
                Self(::std::sync::Arc::clone(&self.0))
            }
        }

        impl<#d: #trait_ident> #trait_ident for #struct_name<#d> {
            #(#items)*
        }
    }
}

/// `impl<T: Transport + Clone + Send + Sync + 'static> Foo for FooClient<T>`.
pub(crate) fn client_trait_impl(
    trait_ident: &syn::Ident,
    client_ident: &syn::Ident,
    methods: &[MethodInfo],
    client: &syn::Path,
) -> TokenStream {
    let items = methods.iter().map(|m| {
        let fn_ident = &m.fn_name;
        let (params, _) = sig_params(m);
        let ev = event_ty(m);
        let const_ident = crate::codegen::client::method_const_ident(m);
        let inserts = m.params.iter().map(|p| {
            crate::codegen::client::param_insert_with(p, client, |err| {
                quote! { return #client::error_events::<#ev>(#err); }
            })
        });
        quote! {
            #[allow(clippy::too_many_arguments)]
            fn #fn_ident(&self #(, #params)*) -> #client::BoxStream<'static, #ev> {
                let mut __params = #client::__rt::serde_json::Map::new();
                #(#inserts)*
                #client::into_events::<#ev, _>(#client::decode::<#ev>(#client::owned_call(
                    ::core::clone::Clone::clone(&self.transport),
                    Self::#const_ident.at(self.path),
                    #client::__rt::serde_json::Value::Object(__params),
                )))
            }
        }
    });
    quote! {
        impl<T> #trait_ident for #client_ident<T>
        where
            T: #client::Transport
                + ::core::clone::Clone
                + ::core::marker::Send
                + ::core::marker::Sync
                + 'static,
        {
            #(#items)*
        }
    }
}

/// `client` in the attribute of an activation the name rule applies to: the
/// client is emitted whenever the crate depends on plexus-client, so the flag
/// says nothing. Accepted for one release, with this warning.
pub(crate) fn client_flag_warning(span: Span, struct_name: &syn::Ident) -> TokenStream {
    let const_name = format_ident!("_PLEXUS_MACROS_DEPRECATED_CLIENT_FLAG_{}", struct_name, span = span);
    let touch = format_ident!("_plexus_macros_touch_deprecated_client_flag_{}", struct_name, span = span);
    quote::quote_spanned! { span =>
        #[doc(hidden)]
        #[allow(non_upper_case_globals, non_snake_case, dead_code)]
        #[deprecated(
            since = "0.6.0",
            note = "plexus-macros (MACRO-13): the `client` flag is ignored — the client is emitted whenever the crate depends on plexus-client. Remove `client` from the attribute; it will be refused in a later release."
        )]
        const #const_name: () = ();

        #[doc(hidden)]
        #[allow(non_snake_case, dead_code)]
        fn #touch() {
            let _ = #const_name;
        }
    }
}

/// The cfg predicate the server half is gated on, parsed.
pub(crate) fn parse_gate(predicate: &str, span: Span) -> syn::Result<syn::Meta> {
    syn::parse_str(predicate).map_err(|e| {
        syn::Error::new(
            span,
            format!(
                "server_cfg = \"...\" must be a `cfg` predicate, e.g. \
                 `feature = \"server\"` or `not(target_arch = \"wasm32\")` ({e})"
            ),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::snake_case;

    #[test]
    fn the_stem_in_snake_case() {
        assert_eq!(snake_case("Echo"), "echo");
        assert_eq!(snake_case("TicketsApi"), "tickets_api");
        assert_eq!(snake_case("HTTPServer"), "http_server");
        assert_eq!(snake_case("Deploy2Driver"), "deploy2_driver");
    }
}
