//! MACRO-13 · `#[activation]` needs nothing; RETURNS-47 · no suffix forced,
//! a trait only when named.
//!
//! On `impl FooActivation` the stem `Foo` names the namespace (`foo`) and the
//! client (`FooClient<T>`); on `impl Foo` (RETURNS-47) the type's own name
//! does; `trait = "Bar"` names the trait, the client and the namespace
//! whatever the type is called. On `impl forge::Forge for GitHub` the trait
//! names the namespace (`forge`) and the client (`ForgeClient<T>`), and the
//! type names nothing. Every `pub async fn` on `&self` returning one of
//! RETURNS-8's signature forms is a wire method (`methods = marked` restores
//! the `#[method]`-only rule); in a trait impl, every `async fn` on `&self`.
//!
//! A trait is emitted only when named (`trait = "…"`), or for a contract (a
//! bodiless activation, which is its trait): each method under its own
//! return type, boxed (RETURNS-36) — `-> T` and `-> Result<T, E>` as
//! `BoxFuture<'static, …>`, every stream form as `BoxStream<'static, item>`
//! — implemented by the activation (and by a contract's struct). The client
//! implements no trait (RETURNS-47, superseding RETURNS-43): its methods
//! answer `Result<T, ClientError>` or a `MethodStream`, so an item type needs
//! no `FromClientError` and `-> u64` is a method like any other. A crate that
//! declares a `server` feature gets its server half gated on it. A bodiless
//! activation is a contract: the macro emits `FooActivation<D: Foo>`,
//! delegating to `D`, and `Foo::server(self)`.
//!
//! An activation written before MACRO-13 — a type not named `…Activation`
//! that says its `namespace` — expands as it did (no trait, the client on the
//! `client` flag, only `#[method]`s on the wire), with a deprecation warning
//! saying what to change, so every repository that takes the macro by path
//! keeps building until it does.

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
    /// The rule applies: every activation but one written before MACRO-13
    /// (a type not named `…Activation` that says its `namespace`, which
    /// expands as it did).
    pub new_form: bool,
    /// The trait the macro emits: named (`trait = "…"`), a contract's (a
    /// bodiless activation is its trait), or — NEST-17 — by default
    /// `{Name}ActivationSchema`. None on `impl Trait for Type` (the trait is
    /// the one implemented) and for an activation written before MACRO-13.
    pub trait_ident: Option<syn::Ident>,
    /// NEST-17: the trait is the default `{Name}ActivationSchema`, asked for
    /// by no one — so it steps aside where it cannot be written (a generic
    /// impl, a crate that can name no boxed stream).
    pub trait_default: bool,
    pub namespace: String,
    pub client_ident: syn::Ident,
}

/// The stem the name rule takes from a type name: `FooActivation` → `Foo`.
pub(crate) fn stem_of(type_name: &str) -> Option<&str> {
    type_name.strip_suffix(SUFFIX).filter(|s| !s.is_empty())
}

impl Names {
    /// `trait_impl`: the trait of `impl Trait for Type`; `contract`: every
    /// wire method is bodiless.
    pub fn resolve(
        args: &HubMethodsAttrs,
        struct_name: &syn::Ident,
        trait_impl: Option<&syn::Path>,
        contract: bool,
    ) -> syn::Result<Self> {
        // RETURNS-47: `impl forge::Forge for GitHub` — the trait names the
        // namespace and the client; the type names nothing.
        if let Some(path) = trait_impl {
            if let Some(lit) = &args.trait_attr {
                return Err(syn::Error::new(
                    lit.span(),
                    format!(
                        "`trait = \"{}\"` on `impl {} for {struct_name}`: the trait is the one this impl \
                         implements, and the macro emits no other. Remove `trait = \"…\"`.",
                        lit.value(),
                        quote!(#path).to_string().replace(' ', ""),
                    ),
                ));
            }
            let stem = path
                .segments
                .last()
                .map(|s| s.ident.clone())
                .ok_or_else(|| syn::Error::new_spanned(path, "expected a trait path"))?;
            return Ok(Names {
                new_form: true,
                namespace: args.namespace.clone().unwrap_or_else(|| snake_case(&stem.to_string())),
                client_ident: format_ident!("{}Client", stem),
                trait_ident: None,
                trait_default: false,
            });
        }

        let type_name = struct_name.to_string();
        let type_stem = stem_of(&type_name).map(|s| syn::Ident::new(s, struct_name.span()));
        let explicit = args
            .trait_attr
            .as_ref()
            .map(|lit| syn::Ident::new(&lit.value(), lit.span()));

        // `trait = "Bar"` names the trait, the client and (absent
        // `namespace`) the namespace, as MACRO-13 had it.
        if let Some(named) = explicit {
            return Ok(Names {
                new_form: true,
                namespace: args.namespace.clone().unwrap_or_else(|| snake_case(&named.to_string())),
                client_ident: format_ident!("{}Client", named),
                trait_ident: Some(named),
                trait_default: false,
            });
        }

        // `impl FooActivation`: the stem, as MACRO-13 had it — a contract's
        // trait is the stem; any other activation's (NEST-17) is
        // `{Stem}ActivationSchema`.
        if let Some(stem) = type_stem {
            return Ok(Names {
                new_form: true,
                namespace: args.namespace.clone().unwrap_or_else(|| snake_case(&stem.to_string())),
                client_ident: format_ident!("{}Client", stem),
                trait_ident: Some(if contract { stem.clone() } else { schema_trait_ident(&stem) }),
                trait_default: !contract,
            });
        }

        // Before MACRO-13 every activation said its namespace; one not named
        // `…Activation` that still does keeps its old expansion, with a
        // deprecation warning (`legacy_name_warning`).
        if let Some(ns) = &args.namespace {
            return Ok(Names {
                new_form: false,
                trait_ident: None,
                trait_default: false,
                namespace: ns.clone(),
                client_ident: format_ident!("{}Client", struct_name),
            });
        }

        // RETURNS-47: `impl Tickets` — the type's own name names it.
        if contract {
            return Err(syn::Error::new(
                struct_name.span(),
                format!(
                    "`{struct_name}` is bodiless, so it is a contract, and a contract is its trait — which \
                     cannot share the type's name. Name the trait with `trait = \"…\"`, name the type \
                     `{struct_name}Activation` (the trait is then `{struct_name}`), or write the trait by \
                     hand and `#[plexus_macros::activation] impl {struct_name} for YourType`."
                ),
            ));
        }
        Ok(Names {
            new_form: true,
            namespace: snake_case(&type_name),
            client_ident: format_ident!("{}Client", struct_name),
            trait_ident: Some(schema_trait_ident(struct_name)),
            trait_default: true,
        })
    }
}

/// NEST-17 (owner, 2026-11-09; NEST-18 D2): the trait every activation gets
/// unless it names one — `Shop` → `ShopActivationSchema`.
pub(crate) fn schema_trait_ident(name: &syn::Ident) -> syn::Ident {
    format_ident!("{}ActivationSchema", name, span = name.span())
}

/// An activation written before MACRO-13 on a type not named
/// `…Activation`, its `namespace` given: it still builds, as it did, and
/// says what to change.
pub(crate) fn legacy_name_warning(struct_name: &syn::Ident) -> TokenStream {
    let span = struct_name.span();
    let const_name = format_ident!("_PLEXUS_MACROS_DEPRECATED_UNNAMED_ACTIVATION_{}", struct_name, span = span);
    let touch = format_ident!("_plexus_macros_touch_unnamed_activation_{}", struct_name, span = span);
    let note = format!(
        "plexus-macros (MACRO-13, RETURNS-47): `{struct_name}` says its `namespace`, so it expands as \
         before MACRO-13 (only `#[method]`s on the wire, the client on the `client` flag). Remove \
         `namespace` — `impl {struct_name}` is named by itself: namespace `{ns}`, client \
         `{struct_name}Client` — or name the type `{struct_name}Activation` to keep `namespace` as an \
         override; then `client`, `crate_path`, `server_cfg` and `#[method]` may go too. A later \
         release refuses it.",
        ns = snake_case(&struct_name.to_string()),
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
/// on the wire? `pub async fn` on `&self` returning `impl Stream<Item = E>`
/// is; one returning anything else is refused (MACRO-13's guard — a
/// one-item method, `-> T`, says so with `#[method]`); any other shape is a
/// plain method.
pub(crate) fn is_wire_shape(method: &ImplItemFn) -> syn::Result<bool> {
    let public = matches!(method.vis, Visibility::Public(_));
    let on_ref_self = matches!(
        method.sig.inputs.first(),
        Some(FnArg::Receiver(r)) if r.reference.is_some() && r.mutability.is_none()
    );
    if !(public && method.sig.asyncness.is_some() && on_ref_self) {
        return Ok(false);
    }
    if let Ok(shape) = classify_return(&method.sig) {
        if shape.cardinality == Cardinality::Many {
            return Ok(true);
        }
    }
    let name = &method.sig.ident;
    let returns = match &method.sig.output {
        syn::ReturnType::Type(_, ty) => quote!(#ty).to_string(),
        syn::ReturnType::Default => "()".to_string(),
    };
    Err(syn::Error::new(
        name.span(),
        format!(
            "`pub async fn {name}(&self, …)` returns `{returns}`, not `impl Stream<Item = E>`, so \
             it cannot go on the wire — and every `pub async fn` on `&self` of an activation is a \
             wire method. Make `{name}` non-`pub`, or write `methods = marked` on the activation \
             (then only #[plexus_macros::method]s are on the wire)."
        ),
    ))
}

/// RETURNS-47: on `impl Trait for Type`, is this method on the wire? Every
/// `async fn` on `&self` is: the trait is the contract, and a trait impl's
/// methods carry no `pub`. Anything else is a plain method.
pub(crate) fn is_trait_wire_shape(method: &ImplItemFn) -> bool {
    let on_ref_self = matches!(
        method.sig.inputs.first(),
        Some(FnArg::Receiver(r)) if r.reference.is_some() && r.mutability.is_none()
    );
    method.sig.asyncness.is_some() && on_ref_self
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

/// What the trait's methods are written in: the boxed stream and future
/// (plexus-client's when the crate has it, else futures', directly or
/// through the umbrella), and the crate `Progress` and `Note` are named
/// through.
pub(crate) struct Boxes {
    pub stream: TokenStream,
    pub future: TokenStream,
    pub vocab: syn::Path,
}

pub(crate) fn boxes(client_path: Option<&syn::Path>, crate_path: &syn::Path, span: Span) -> syn::Result<Boxes> {
    let vocab: syn::Path = match client_path {
        Some(client) => client.clone(),
        None => syn::parse_quote!(#crate_path::plexus),
    };
    if let Some(client) = client_path {
        return Ok(Boxes { stream: quote! { #client::BoxStream }, future: quote! { #client::BoxFuture }, vocab });
    }
    if let Ok(found) = crate_name("futures") {
        let ident = match found {
            FoundCrate::Itself => format_ident!("futures"),
            FoundCrate::Name(n) => syn::Ident::new(&n, span),
        };
        return Ok(Boxes {
            stream: quote! { ::#ident::stream::BoxStream },
            future: quote! { ::#ident::future::BoxFuture },
            vocab,
        });
    }
    if let Some(umbrella) = crate::codegen::umbrella_crate(span) {
        return Ok(Boxes {
            stream: quote! { ::#umbrella::__deps::futures::stream::BoxStream },
            future: quote! { ::#umbrella::__deps::futures::future::BoxFuture },
            vocab,
        });
    }
    Err(syn::Error::new(
        span,
        "#[plexus_macros::activation]: the activation's trait returns boxed streams and futures, and \
         this crate depends on neither `plexus-client` nor `futures` to name them. Add \
         `plexus-client` (which also gives you the client) or `futures`.",
    ))
}

/// RETURNS-36: a method's item as its signature has it — `T`,
/// `Result<T, E>`, `Progress<T, P>`, `Result<Progress<T, P>, E>`.
fn item_ty(m: &MethodInfo, vocab: &syn::Path) -> TokenStream {
    let ok = &m.ok_ty;
    let inner = match &m.progress_ty {
        Some(p) => {
            let p = p.to_type(vocab);
            quote! { #vocab::Progress<#ok, #p> }
        }
        None => quote! { #ok },
    };
    match &m.err_ty {
        Some(e) => quote! { ::core::result::Result<#inner, #e> },
        None => inner,
    }
}

/// RETURNS-36: the trait method's return type — the method's own, boxed: a
/// future of its item for a one-item method, a stream of its items else.
fn ret_ty(m: &MethodInfo, b: &Boxes) -> TokenStream {
    let item = item_ty(m, &b.vocab);
    match m.cardinality {
        Cardinality::One => {
            let f = &b.future;
            quote! { #f<'static, #item> }
        }
        Cardinality::Many => {
            let s = &b.stream;
            quote! { #s<'static, #item> }
        }
    }
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
    } else {
        None
    }
}

/// Does a method take what the server injects? A contract may not (its
/// methods are what its implementor is called with); an activation with
/// such a method implements its own trait by reading them from the call's
/// context (NEST-17), [`self_implementable`].
pub(crate) fn self_callable(methods: &[MethodInfo]) -> bool {
    methods.iter().all(|m| injected(m).is_none())
}

/// NEST-17: whether reading a method's injected values from the call's
/// context can fail — a caller required (`auth: &AuthContext`, a
/// `#[from_auth]` resolver) and none there. `auth: Option<&AuthContext>`
/// is `None` and `reach` is `Reach::nowhere()` outside a call: fail closed,
/// never failing.
fn caller_can_fail(m: &MethodInfo) -> bool {
    m.requires_auth && !(m.auth_optional && m.auth_resolvers.is_empty())
}

/// NEST-17: can the activation implement its own trait? Every method a
/// caller could call on the trait must be callable from it: one that takes
/// what the server injects reads it from the call's context
/// (`plexus_core::call::caller()`, as `impl Trait for Type` does) — except
/// an `#[activation_param]`, which comes from the transport's request and
/// is in no context; a generic impl's methods (whose missing-caller error
/// cannot be named once for every instance); and a one-item method that
/// needs a caller and answers no `Result` (it has nowhere to say the caller
/// is missing).
pub(crate) fn self_implementable(methods: &[MethodInfo], generic: bool) -> bool {
    methods.iter().all(|m| {
        if injected(m).is_none() {
            return true;
        }
        !generic
            && m.activation_params.is_empty()
            && !(m.cardinality == Cardinality::One && m.err_ty.is_none() && caller_can_fail(m))
    })
}

/// A trait method's parameters and their names. BIDIR-6: an `Input<T>` is a
/// boxed stream of `T` in the trait, which its implementors all take: the
/// activation as its `Input`, the client as what it sends.
fn sig_params<'a>(m: &'a MethodInfo, box_stream: &TokenStream) -> (Vec<TokenStream>, Vec<&'a syn::Ident>) {
    let mut params: Vec<TokenStream> = m
        .params
        .iter()
        .map(|p| {
            let (name, ty) = (&p.name, &p.ty);
            quote! { #name: #ty }
        })
        .collect();
    let mut names: Vec<&syn::Ident> = m.params.iter().map(|p| &p.name).collect();
    if let Some(i) = &m.input {
        let (name, item) = (&i.name, &i.item);
        params.insert(i.at, quote! { #name: #box_stream<'static, #item> });
        names.insert(i.at, name);
    }
    (params, names)
}

/// The names as the inherent method and the trait method pass them to one
/// another: an `Input` through `convert`.
fn passed<'a>(m: &'a MethodInfo, convert: impl Fn(&syn::Ident) -> TokenStream) -> Vec<TokenStream> {
    let mut names: Vec<TokenStream> = m.params.iter().map(|p| { let n = &p.name; quote! { #n } }).collect();
    if let Some(i) = &m.input {
        names.insert(i.at, convert(&i.name));
    }
    names
}

/// `pub trait Foo: Send + Sync + 'static { fn m(&self, …) -> <m's own return
/// type, boxed>; … }`, its dyn-compatibility assertion, and — for a contract
/// — `server()`.
pub(crate) fn trait_def(
    trait_ident: &syn::Ident,
    struct_name: &syn::Ident,
    description: &str,
    methods: &[MethodInfo],
    boxes: &Boxes,
    contract: bool,
    server_gate: Option<&syn::Meta>,
) -> TokenStream {
    let items = methods.iter().map(|m| {
        let fn_ident = &m.fn_name;
        let (params, _) = sig_params(m, &boxes.stream);
        let ret = ret_ty(m, boxes);
        let doc = &m.description;
        let doc = if doc.is_empty() { quote! {} } else { quote! { #[doc = #doc] } };
        quote! {
            #doc
            #[allow(clippy::too_many_arguments)]
            fn #fn_ident(&self #(, #params)*) -> #ret;
        }
    });
    let server = if contract {
        let gate = server_gate.map(|m| quote! { #[cfg(#m)] });
        quote! {
            /// Serve this implementation on a service: `service.register(x.server())`.
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
        /// Emitted by `#[plexus_macros::activation]` from the activation's
        /// wire methods (NEST-17: for every activation, named
        /// `{Name}ActivationSchema` unless `trait = "…"` names it; a contract
        /// is its trait), each under its own return type (RETURNS-36), the
        /// values the server injects left out: implemented by the activation
        /// (reading those from the call's context), and by its client when
        /// every method answers a `Result` whose error takes a `PeerError`,
        /// so a local activation and a remote one are interchangeable behind
        /// `dyn` this trait.
        #[allow(private_interfaces)]
        pub trait #trait_ident: ::core::marker::Send + ::core::marker::Sync + 'static {
            #(#items)*
            #server
        }

        // MACRO-13: the trait is dyn-compatible by construction (boxed
        // streams and futures, no generics, no `async fn`); this fails to
        // compile the day it is not.
        #[doc(hidden)]
        #[allow(dead_code, non_snake_case)]
        fn #assert_fn(_: &dyn #trait_ident) {}
    }
}

/// Server half, server form: `impl Foo for FooActivation`, each method the
/// inherent method's stream. The trait's stream is `'static` and the inherent
/// method's future borrows `self`, so the activation is cloned into the
/// stream (`Plexus::register` already asks every activation to be `Clone`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn activation_trait_impl(
    trait_ident: &syn::Ident,
    struct_name: &syn::Ident,
    self_ty: &Type,
    impl_generics: &syn::ImplGenerics,
    where_clause: Option<&syn::WhereClause>,
    methods: &[MethodInfo],
    boxes: &Boxes,
    deps: &syn::Ident,
    guarded: Option<(&syn::Path, &str)>,
) -> TokenStream {
    if let Some((crate_path, namespace)) = guarded {
        return guarded_trait_impl(trait_ident, self_ty, methods, boxes, deps, crate_path, namespace);
    }
    let items = methods.iter().map(|m| {
        let fn_ident = &m.fn_name;
        let (params, _) = sig_params(m, &boxes.stream);
        let names = passed(m, |n| quote! { ::core::convert::Into::into(#n) });
        let ret = ret_ty(m, boxes);
        let body = match m.cardinality {
            Cardinality::One => quote! {
                ::std::boxed::Box::pin(async move { #struct_name::#fn_ident(&__this #(, #names)*).await })
            },
            Cardinality::Many => quote! {
                ::std::boxed::Box::pin(#deps::futures::StreamExt::flatten(
                    #deps::futures::stream::once(async move {
                        #struct_name::#fn_ident(&__this #(, #names)*).await
                    }),
                ))
            },
        };
        quote! {
            #[allow(clippy::too_many_arguments)]
            fn #fn_ident(&self #(, #params)*) -> #ret {
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
                #body
            }
        }
    });
    quote! {
        impl #impl_generics #trait_ident for #self_ty #where_clause {
            #(#items)*
        }
    }
}

/// NEST-17: `impl Trait for Type` where the activation is `Clone` (a
/// higher-ranked bound, so a type that is not simply has no impl, rather
/// than an error: the trait is emitted for every activation, unasked), each
/// method that takes what the server injects through a hidden inherent
/// helper reading it from the call's context.
fn guarded_trait_impl(
    trait_ident: &syn::Ident,
    self_ty: &Type,
    methods: &[MethodInfo],
    boxes: &Boxes,
    deps: &syn::Ident,
    crate_path: &syn::Path,
    namespace: &str,
) -> TokenStream {
    let mut helpers = Vec::new();
    let items = methods.iter().map(|m| {
        let fn_ident = &m.fn_name;
        let (params, names) = sig_params(m, &boxes.stream);
        let ret = ret_ty(m, boxes);
        let item = item_ty(m, &boxes.vocab);
        let body = if injected(m).is_none() {
            let passed = passed(m, |n| quote! { ::core::convert::Into::into(#n) });
            match m.cardinality {
                Cardinality::One => quote! {
                    ::std::boxed::Box::pin(async move { Self::#fn_ident(&__this #(, #passed)*).await })
                },
                Cardinality::Many => quote! {
                    ::std::boxed::Box::pin(#deps::futures::StreamExt::flatten(
                        #deps::futures::stream::once(async move { Self::#fn_ident(&__this #(, #passed)*).await }),
                    ))
                },
            }
        } else {
            let helper = format_ident!("__plexus_schema_{}", fn_ident);
            helpers.push(context_helper(m, &helper, &params, boxes, crate_path));
            let grant = format!("{namespace}.{}", m.method_name);
            let fail = m.err_ty.as_ref().map(|e| fail_into(e, &grant, crate_path));
            match m.cardinality {
                Cardinality::One => {
                    // Without a `Result` only a caller that cannot be missing
                    // gets here (`self_implementable`): the helper never fails.
                    let on_fail = match fail {
                        Some(f) => quote! { ::core::result::Result::Err(#f) },
                        None => quote! { ::core::unreachable!("`{}` cannot miss its caller: {}", #grant, __e) },
                    };
                    quote! {
                        ::std::boxed::Box::pin(async move {
                            match __this.#helper(#(#names),*).await {
                                ::core::result::Result::Ok(__v) => __v,
                                ::core::result::Result::Err(__e) => #on_fail,
                            }
                        })
                    }
                }
                Cardinality::Many => {
                    let stream = &boxes.stream;
                    let on_fail = match fail {
                        Some(f) => quote! {
                            let __item: #item = ::core::result::Result::Err(#f);
                            ::std::boxed::Box::pin(#deps::futures::stream::once(::core::future::ready(__item)))
                        },
                        // No error to say it in: fail closed, nothing.
                        None => quote! { { let _ = __e; ::std::boxed::Box::pin(#deps::futures::stream::empty()) } },
                    };
                    quote! {
                        ::std::boxed::Box::pin(#deps::futures::StreamExt::flatten(
                            #deps::futures::stream::once(async move {
                                let __s: #stream<'static, #item> = match __this.#helper(#(#names),*).await {
                                    ::core::result::Result::Ok(__s) => __s,
                                    ::core::result::Result::Err(__e) => { #on_fail }
                                };
                                __s
                            }),
                        ))
                    }
                }
            }
        };
        quote! {
            #[allow(clippy::too_many_arguments)]
            fn #fn_ident(&self #(, #params)*) -> #ret {
                let __this = <#self_ty as ::core::clone::Clone>::clone(self);
                #body
            }
        }
    }).collect::<Vec<_>>();
    quote! {
        // NEST-17: what a trait method that needs its caller reads from the
        // call's context; used by the trait's impl alone.
        #[allow(dead_code, non_snake_case, clippy::too_many_arguments)]
        impl #self_ty {
            #(#helpers)*
        }

        impl #trait_ident for #self_ty
        where
            for<'__plexus_schema> #self_ty: ::core::clone::Clone,
        {
            #(#items)*
        }
    }
}

/// NEST-17: the hidden helper of one method that takes what the server
/// injects: those values from the call's context, then the method —
/// `Err(PlexusError)` when a caller is required and none is there.
fn context_helper(
    m: &MethodInfo,
    helper: &syn::Ident,
    params: &[TokenStream],
    boxes: &Boxes,
    crate_path: &syn::Path,
) -> TokenStream {
    let fn_ident = &m.fn_name;
    let item = item_ty(m, &boxes.vocab);
    let stream = &boxes.stream;
    let out = match m.cardinality {
        Cardinality::One => quote! { #item },
        Cardinality::Many => quote! { #stream<'static, #item> },
    };
    let plexus_error = quote! { #crate_path::plexus::PlexusError };
    let auth = if !m.requires_auth {
        quote! {}
    } else if m.auth_optional && m.auth_resolvers.is_empty() {
        quote! {
            let __plexus_caller = #crate_path::call::caller().ok();
            let auth_ctx: ::core::option::Option<&#crate_path::plexus::AuthContext> =
                __plexus_caller.as_ref().and_then(|__c| __c.auth());
        }
    } else {
        quote! {
            let __plexus_caller = #crate_path::call::caller().map_err(<#plexus_error as ::core::convert::From<_>>::from)?;
            let auth_ctx: &#crate_path::plexus::AuthContext = __plexus_caller.signed_in()?;
        }
    };
    let resolvers: Vec<TokenStream> = m
        .auth_resolvers
        .iter()
        .map(|r| crate::codegen::activation::resolve_typed(&r.param_name, &r.resolver_expr, crate_path))
        .collect();
    let reach = if m.takes_reach {
        quote! {
            let __plexus_reach = match #crate_path::plexus::current_reach() {
                ::core::result::Result::Ok(__r) => __r,
                ::core::result::Result::Err(_) => #crate_path::plexus::Reach::nowhere(),
            };
        }
    } else {
        quote! {}
    };
    let reach_arg = if m.takes_reach { quote! { __plexus_reach, } } else { quote! {} };
    let names = passed(m, |n| quote! { ::core::convert::Into::into(#n) });
    let resolver_names: Vec<&syn::Ident> = m.auth_resolvers.iter().map(|r| &r.param_name).collect();
    let args = if m.requires_auth && m.auth_resolvers.is_empty() {
        quote! { auth_ctx, #reach_arg #(#names),* }
    } else {
        quote! { #reach_arg #(#resolver_names,)* #(#names),* }
    };
    let call = match m.cardinality {
        Cardinality::One => quote! { Self::#fn_ident(self, #args).await },
        Cardinality::Many => quote! { ::std::boxed::Box::pin(Self::#fn_ident(self, #args).await) },
    };
    quote! {
        #[doc(hidden)]
        async fn #helper(&self #(, #params)*) -> ::core::result::Result<#out, #plexus_error> {
            #auth
            #(#resolvers)*
            #reach
            ::core::result::Result::Ok(#call)
        }
    }
}

/// NEST-17: a missing caller as the method's own error `E`, by what `E`
/// takes — a `PlexusError`, else a `PeerError` (`forbidden` naming the
/// method's grant when no one is signed in, `unavailable` else) — chosen by
/// autoref on `E`; an `E` that takes neither panics, naming the method:
/// there is no value to answer with.
fn fail_into(err: &Type, grant: &str, crate_path: &syn::Path) -> TokenStream {
    let plexus_error = quote! { #crate_path::plexus::PlexusError };
    let peer_error = quote! { #crate_path::plexus::peer::PeerError };
    quote! {{
        struct __PlexusFail<E>(::core::cell::Cell<::core::option::Option<#plexus_error>>, ::core::marker::PhantomData<E>);
        trait __PlexusViaPlexus<E> { fn __plexus_fail(&self) -> E; }
        impl<E: ::core::convert::From<#plexus_error>> __PlexusViaPlexus<E> for &__PlexusFail<E> {
            fn __plexus_fail(&self) -> E {
                <E as ::core::convert::From<#plexus_error>>::from(self.0.take().expect("taken once"))
            }
        }
        trait __PlexusViaPeer<E> { fn __plexus_fail(&self) -> E; }
        impl<E: ::core::convert::From<#peer_error>> __PlexusViaPeer<E> for &&__PlexusFail<E> {
            fn __plexus_fail(&self) -> E {
                let __e = self.0.take().expect("taken once");
                let __p = if ::core::matches!(__e, #plexus_error::Unauthenticated(_)) {
                    #peer_error::Forbidden { grant: ::std::string::ToString::to_string(#grant) }
                } else {
                    #peer_error::Unavailable { reason: ::std::string::ToString::to_string(&__e) }
                };
                <E as ::core::convert::From<#peer_error>>::from(__p)
            }
        }
        trait __PlexusViaNothing<E> { fn __plexus_fail(&self) -> E; }
        impl<E> __PlexusViaNothing<E> for __PlexusFail<E> {
            fn __plexus_fail(&self) -> E {
                let __e = self.0.take().expect("taken once");
                ::core::panic!(
                    "`{}` needs its caller, and has none ({}); its error type takes neither a PlexusError nor a PeerError to say so — call it inside a Plexus call",
                    #grant, __e
                )
            }
        }
        (&&__PlexusFail::<#err>(::core::cell::Cell::new(::core::option::Option::Some(__e)), ::core::marker::PhantomData)).__plexus_fail()
    }}
}

/// NEST-17: the client as the activation's trait, for an activation every
/// method of which answers one `Result<T, E>` (as a contract's peer half
/// asks): a call that fails in transit is `E::from(PeerError)`. Bounded on
/// every `E: From<PeerError>` (higher-ranked, so an `E` that takes none
/// leaves the client without the impl, not the crate without a build).
pub(crate) fn client_trait_impl(
    trait_ident: &syn::Ident,
    client_ident: &syn::Ident,
    methods: &[MethodInfo],
    future: &TokenStream,
    client_path: &syn::Path,
    crate_path: &syn::Path,
) -> TokenStream {
    let peer_error = quote! { #crate_path::plexus::peer::PeerError };
    let mut seen = std::collections::HashSet::new();
    let bounds: Vec<TokenStream> = methods
        .iter()
        .filter_map(|m| m.err_ty.as_ref())
        .filter(|e| seen.insert(quote!(#e).to_string()))
        .map(|e| quote! { for<'__plexus_schema> #e: ::core::convert::From<#peer_error>, })
        .collect();
    let items = methods.iter().map(|m| {
        let fn_ident = &m.fn_name;
        let params: Vec<TokenStream> = m.params.iter().map(|p| { let (n, t) = (&p.name, &p.ty); quote! { #n: #t } }).collect();
        let names: Vec<&syn::Ident> = m.params.iter().map(|p| &p.name).collect();
        let ok = &m.ok_ty;
        let err = m.err_ty.as_ref().expect("contract-shaped: every method is fallible");
        quote! {
            #[allow(clippy::too_many_arguments)]
            fn #fn_ident(&self #(, #params)*) -> #future<'static, ::core::result::Result<#ok, #err>> {
                let __client = ::core::clone::Clone::clone(self);
                ::std::boxed::Box::pin(async move {
                    match #client_ident::#fn_ident(&__client #(, #names)*).await {
                        ::core::result::Result::Ok(__inner) => __inner,
                        ::core::result::Result::Err(__e) => ::core::result::Result::Err(
                            <#err as ::core::convert::From<#peer_error>>::from(#peer_error::from_client(&__e)),
                        ),
                    }
                })
            }
        }
    });
    quote! {
        impl<__T> #trait_ident for #client_ident<__T>
        where
            __T: #client_path::Transport + ::core::clone::Clone + ::core::marker::Send + ::core::marker::Sync + 'static,
            #(#bounds)*
        {
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
                let names = passed(m, |n| quote! { #n.into_boxed() });
                f.block = match m.cardinality {
                    // RETURNS-36: the trait's future, awaited in the
                    // inherent `async fn`.
                    Cardinality::One => syn::parse_quote!({
                        <#d as #trait_ident>::#fn_ident(&*self.0 #(, #names)*).await
                    }),
                    Cardinality::Many => syn::parse_quote!({
                        <#d as #trait_ident>::#fn_ident(&*self.0 #(, #names)*)
                    }),
                };
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
    boxes: &Boxes,
) -> TokenStream {
    let d = contract_param();
    let doc = format!(
        "The `{trait_ident}` contract served: an activation that delegates every wire method to \
         its implementor. Emitted by `#[plexus_macros::activation]` (MACRO-13) for a bodiless \
         activation; make one with `{trait_ident}::server(x)` or `{struct_name}::new(x)`."
    );
    let items = methods.iter().map(|m| {
        let fn_ident = &m.fn_name;
        let (params, names) = sig_params(m, &boxes.stream);
        let ret = ret_ty(m, boxes);
        quote! {
            #[allow(clippy::too_many_arguments)]
            fn #fn_ident(&self #(, #params)*) -> #ret {
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

/// RETURNS-36: `#[method(streaming)]` on a stream form — a deprecated no-op,
/// said as a deprecation warning at the attribute.
pub(crate) fn streaming_warnings(methods: &[MethodInfo], struct_name: &syn::Ident) -> TokenStream {
    methods
        .iter()
        .filter_map(|m| {
            let span = m.streaming_deprecated?;
            let const_name = format_ident!("_PLEXUS_MACROS_DEPRECATED_STREAMING_{}_{}", struct_name, m.fn_name, span = span);
            let touch = format_ident!("_plexus_macros_touch_streaming_{}_{}", struct_name, m.fn_name, span = span);
            let note = crate::parse::STREAMING_DEPRECATED;
            Some(quote::quote_spanned! { span =>
                #[doc(hidden)]
                #[allow(non_upper_case_globals, non_snake_case, dead_code)]
                #[deprecated(since = "0.8.0", note = #note)]
                const #const_name: () = ();

                #[doc(hidden)]
                #[allow(non_snake_case, dead_code)]
                fn #touch() {
                    let _ = #const_name;
                }
            })
        })
        .collect()
}

/// NEST-17: `list = "…"` / `search = "…"` on a `#[child]` or `#[peer]` —
/// a string where a path goes now — said as a deprecation warning at the
/// string, for one release.
pub(crate) fn string_arg_warnings(children: &[crate::parse::ChildMethodInfo], struct_name: &syn::Ident) -> TokenStream {
    children
        .iter()
        .flat_map(|c| c.string_args.iter().map(move |(key, span)| (c, key, *span)))
        .map(|(c, key, span)| {
            let const_name = format_ident!("_PLEXUS_MACROS_DEPRECATED_STRING_{}_{}_{}", struct_name, c.fn_name, key, span = span);
            let touch = format_ident!("_plexus_macros_touch_string_{}_{}_{}", struct_name, c.fn_name, key, span = span);
            let method = match key.as_str() {
                "list" => c.list_fn.as_ref(),
                _ => c.search_fn.as_ref(),
            }
            .map(|i| i.to_string())
            .unwrap_or_default();
            let note = format!(
                "plexus-macros (NEST-17): `{key} = \"{method}\"` names a sibling method with a string; write \
                 the path, `{key} = {method}`. The string form is refused in a later release."
            );
            quote::quote_spanned! { span =>
                #[doc(hidden)]
                #[allow(non_upper_case_globals, non_snake_case, dead_code)]
                #[deprecated(since = "0.11.0", note = #note)]
                const #const_name: () = ();

                #[doc(hidden)]
                #[allow(non_snake_case, dead_code)]
                fn #touch() {
                    let _ = #const_name;
                }
            }
        })
        .collect()
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
