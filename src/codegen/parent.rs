//! NEST-16 · a graft that knows its parent: `#[activation(parent = ShopClient)]`.
//!
//! A method takes the parent's client as `parent: Parent<ShopClient>`; the
//! macro writes its transport (`Parent<ShopClient<ParentTransport>>`),
//! injects it at dispatch (and in the trait form, from the call's context),
//! and reads the body for the call chains rooted at it — `parent.items()`,
//! `parent.ui().slots().list()` — which become the activation's `needs`.
//! The parent's client says which link of a chain is its method
//! (`ParentShape`), at the activation's first `schema`.
//!
//! `parent` must stay a chain's root: passed to a function, bound to another
//! name, cloned, its client or transport taken, read inside a macro the
//! macro cannot parse — each is a compile error at that use, unless the
//! method lists what it calls by hand in `uses(…)`.

use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::visit::Visit;
use syn::{Expr, FnArg, Type};

use crate::parse::MethodInfo;

/// Methods of `Parent` and of a generated client that hand out the client or
/// its transport: a chain through one is the parent escaping.
const ESCAPES: &[&str] = &[
    "client", "into_inner", "transport", "into_transport", "clone", "to_owned", "deref", "as_ref", "borrow", "path", "lifted",
];

/// `Parent<C>` → `Parent<C<ParentTransport>>` when `C` is written bare.
pub(crate) fn rewrite_sig(sig: &mut syn::Signature, crate_path: &syn::Path) {
    for arg in sig.inputs.iter_mut() {
        let FnArg::Typed(pt) = arg else { continue };
        let Type::Path(tp) = &mut *pt.ty else { continue };
        let Some(last) = tp.path.segments.last_mut() else { continue };
        if last.ident != "Parent" {
            continue;
        }
        let syn::PathArguments::AngleBracketed(args) = &mut last.arguments else { continue };
        if args.args.len() != 1 {
            continue;
        }
        if let Some(syn::GenericArgument::Type(Type::Path(inner))) = args.args.first_mut() {
            if let Some(seg) = inner.path.segments.last_mut() {
                if matches!(seg.arguments, syn::PathArguments::None) {
                    seg.arguments = syn::PathArguments::AngleBracketed(
                        syn::parse_quote! { <#crate_path::plexus::ParentTransport> },
                    );
                }
            }
        }
    }
}

struct Walk<'a> {
    parent: &'a syn::Ident,
    chains: Vec<Vec<String>>,
    escapes: Vec<(Span, String)>,
}

impl Walk<'_> {
    fn is_parent(&self, e: &Expr) -> bool {
        match e {
            Expr::Path(p) => p.qself.is_none() && p.path.is_ident(self.parent),
            Expr::Paren(p) => self.is_parent(&p.expr),
            _ => false,
        }
    }

    fn escape(&mut self, span: Span, how: String) {
        self.escapes.push((span, how));
    }
}

impl<'ast> Visit<'ast> for Walk<'_> {
    fn visit_expr(&mut self, e: &'ast Expr) {
        match e {
            Expr::MethodCall(_) => {
                // The chain under this call, outermost link first.
                let mut links: Vec<&'ast syn::ExprMethodCall> = Vec::new();
                let mut at: &'ast Expr = e;
                while let Expr::MethodCall(mc) = at {
                    links.push(mc);
                    at = &mc.receiver;
                }
                if self.is_parent(at) {
                    links.reverse();
                    let names: Vec<String> = links.iter().map(|l| l.method.to_string()).collect();
                    let head = &links[0];
                    if ESCAPES.contains(&names[0].as_str()) {
                        self.escape(head.method.span(), format!("`parent.{}()` hands the parent's client out of the call chain", names[0]));
                    } else if names[0] != "host" {
                        self.chains.push(names);
                    }
                    for l in &links {
                        for a in &l.args {
                            self.visit_expr(a);
                        }
                    }
                    return;
                }
                syn::visit::visit_expr(self, e);
            }
            Expr::Path(_) if self.is_parent(e) => {
                self.escape(syn::spanned::Spanned::span(e), "`parent` is used here other than as the root of a call chain".into());
            }
            Expr::Field(f) if self.is_parent(&f.base) => {
                self.escape(syn::spanned::Spanned::span(&f.member), "`parent`'s field is read here".into());
            }
            _ => syn::visit::visit_expr(self, e),
        }
    }

    fn visit_macro(&mut self, m: &'ast syn::Macro) {
        let parser = syn::punctuated::Punctuated::<Expr, syn::Token![,]>::parse_terminated;
        match syn::parse::Parser::parse2(parser, m.tokens.clone()) {
            Ok(exprs) => {
                for e in &exprs {
                    self.visit_expr(e);
                }
            }
            Err(_) => {
                let named = |ts: &TokenStream| -> Option<Span> {
                    fn find(ts: &TokenStream, p: &syn::Ident) -> Option<Span> {
                        for tt in ts.clone() {
                            match tt {
                                proc_macro2::TokenTree::Ident(i) if i == *p => return Some(i.span()),
                                proc_macro2::TokenTree::Group(g) => {
                                    if let Some(s) = find(&g.stream(), p) {
                                        return Some(s);
                                    }
                                }
                                _ => {}
                            }
                        }
                        None
                    }
                    find(ts, self.parent)
                };
                if let Some(span) = named(&m.tokens) {
                    self.escape(span, "`parent` is used inside a macro whose arguments are not expressions".into());
                }
            }
        }
    }

    fn visit_pat_ident(&mut self, p: &'ast syn::PatIdent) {
        // `let parent = …` shadows it: what follows is not the parent.
        if p.ident == *self.parent {
            self.escape(p.ident.span(), "`parent` is bound again here".into());
        }
        syn::visit::visit_pat_ident(self, p);
    }
}

/// Read `m`'s body for the call chains on its `parent`; a use that is not a
/// chain's root is a compile error naming where, unless `m` says `uses(…)`.
pub(crate) fn infer(m: &mut MethodInfo, body: &syn::Block) -> syn::Result<()> {
    let Some(p) = &m.parent else { return Ok(()) };
    let mut w = Walk { parent: &p.name, chains: Vec::new(), escapes: Vec::new() };
    w.visit_block(body);
    if m.uses.is_empty() {
        if let Some((span, how)) = w.escapes.into_iter().next() {
            return Err(syn::Error::new(
                span,
                format!(
                    "`{}`: {how}, so what it calls of its parent cannot be read from the code. Call the parent's \
                     methods on `{}` itself (`{}.items()`), or list what this method calls by hand: \
                     `#[plexus_macros::method(uses(\"shop.items\"))]`",
                    m.fn_name, p.name, p.name
                ),
            ));
        }
    }
    w.chains.sort();
    w.chains.dedup();
    m.parent_chains = w.chains;
    Ok(())
}

/// The activation's hidden `__plexus_needs()` (its schema's `needs`, every
/// method's chains and `uses` resolved through the parent's client) and
/// `__plexus_parent_needs()` (their paths, what [`ParentTransport`] lets
/// through), or nothing when it names no parent.
pub(crate) fn needs_impl(
    parent: Option<&syn::Path>,
    methods: &[MethodInfo],
    self_ty: &Type,
    impl_generics: &syn::ImplGenerics,
    where_clause: Option<&syn::WhereClause>,
    crate_path: &syn::Path,
    client_path: Option<&syn::Path>,
) -> syn::Result<TokenStream> {
    let Some(parent) = parent else {
        if let Some(m) = methods.iter().find(|m| m.parent.is_some()) {
            return Err(syn::Error::new(
                m.fn_name.span(),
                format!(
                    "`{}` takes a `Parent`, but the activation names no parent: \
                     `#[plexus_macros::activation(parent = ShopClient)]`",
                    m.fn_name
                ),
            ));
        }
        if let Some(m) = methods.iter().find(|m| !m.uses.is_empty()) {
            return Err(syn::Error::new(
                m.fn_name.span(),
                format!("`{}` says `uses(…)`, but the activation names no parent to use", m.fn_name),
            ));
        }
        return Ok(TokenStream::new());
    };
    let Some(client) = client_path else {
        return Err(syn::Error::new_spanned(parent, "parent = …: the activation's crate depends on no plexus-client, so it has no client to call its parent with"));
    };
    let parent_name = parent.segments.last().map(|s| s.ident.to_string()).unwrap_or_default();
    for m in methods {
        if let Some(p) = &m.parent {
            let written = match &p.client {
                Type::Path(tp) => tp.path.segments.last().map(|s| s.ident.to_string()).unwrap_or_default(),
                _ => String::new(),
            };
            if written != parent_name {
                return Err(syn::Error::new_spanned(
                    &p.ty,
                    format!("the activation's parent is `{parent_name}`, so a method takes `Parent<{parent_name}>`"),
                ));
            }
        }
    }
    let mut parent_ty = parent.clone();
    if let Some(seg) = parent_ty.segments.last_mut() {
        seg.arguments = syn::PathArguments::AngleBracketed(syn::parse_quote! { <()> });
    }
    let mut chains: Vec<Vec<String>> = methods.iter().flat_map(|m| m.parent_chains.iter().cloned()).collect();
    chains.sort();
    chains.dedup();
    let chains = chains.iter().map(|c| quote! { &[#(#c),*] });
    let mut uses: Vec<String> = methods.iter().flat_map(|m| m.uses.iter().cloned()).collect();
    uses.sort();
    uses.dedup();
    let need = quote! { #crate_path::plexus::Need };
    Ok(quote! {
        #[allow(dead_code)]
        impl #impl_generics #self_ty #where_clause {
            /// NEST-16: what this activation calls of its parent — its
            /// schema's `needs`.
            #[doc(hidden)]
            pub fn __plexus_needs() -> ::std::vec::Vec<#need> {
                static __NEEDS: ::std::sync::OnceLock<::std::vec::Vec<#need>> = ::std::sync::OnceLock::new();
                ::core::clone::Clone::clone(__NEEDS.get_or_init(|| {
                    let __ns = <#parent_ty as #client::ParentShape>::namespace();
                    let __chains: &[&[&str]] = &[#(#chains),*];
                    let __uses: &[&str] = &[#(#uses),*];
                    let mut __out: ::std::vec::Vec<#need> = ::std::vec::Vec::new();
                    for __c in __chains {
                        __out.push(match <#parent_ty as #client::ParentShape>::resolve(__c) {
                            ::core::option::Option::Some((__p, __h)) => #need::new(::std::format!("{}.{}", __ns, __p), __h.unwrap_or_default()),
                            // Not a method of the parent's client (a dynamic
                            // child's accessor): named as written, so the
                            // graft refuses it naming it — say it in `uses`.
                            ::core::option::Option::None => #need::new(::std::format!("{}.{}", __ns, __c[0]), ""),
                        });
                    }
                    for __u in __uses {
                        let __h = __u
                            .strip_prefix(__ns)
                            .and_then(|__r| __r.strip_prefix('.'))
                            .and_then(|__r| <#parent_ty as #client::ParentShape>::resolve(&__r.split('.').collect::<::std::vec::Vec<_>>()))
                            .filter(|(__p, _)| ::std::format!("{}.{}", __ns, __p) == *__u)
                            .and_then(|(_, __h)| __h)
                            .unwrap_or_default();
                        __out.push(#need::new(*__u, __h));
                    }
                    __out.sort();
                    __out.dedup_by(|a, b| a.method == b.method);
                    __out
                }))
            }

            /// NEST-16: the paths of [`Self::__plexus_needs`], what the
            /// injected parent's transport lets through.
            #[doc(hidden)]
            pub fn __plexus_parent_needs() -> ::std::sync::Arc<[::std::string::String]> {
                static __PATHS: ::std::sync::OnceLock<::std::sync::Arc<[::std::string::String]>> = ::std::sync::OnceLock::new();
                ::std::sync::Arc::clone(__PATHS.get_or_init(|| {
                    Self::__plexus_needs().into_iter().map(|n| n.method).collect::<::std::vec::Vec<_>>().into()
                }))
            }
        }
    })
}

/// The dispatch's (and the trait form's) binding of the injected `parent`.
pub(crate) fn binding(m: &MethodInfo, namespace: &str, crate_path: &syn::Path) -> TokenStream {
    let Some(p) = &m.parent else { return TokenStream::new() };
    let (name, client) = (&p.name, &p.client);
    quote! {
        let #name = #crate_path::plexus::Parent::__in_call(#namespace, Self::__plexus_parent_needs(), |__t| <#client>::new(__t))?;
    }
}
