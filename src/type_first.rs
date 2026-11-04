//! PERF.115-43 · `#[derive(plexus_macros::Deserialize)]`: an internally
//! tagged enum's `Deserialize`, its tag read first.
//!
//! serde reads an internally tagged enum whole into a private buffer before
//! it reads the tag. This derive gives the enum a `Deserialize` that reads
//! the first key and, when it is the tag, reads the variant straight from the
//! rest of the object (plexus-client's `__rt::type_first` does the reading).
//! What it cannot read straight, serde reads, buffer and all.
//!
//! It emits, in a `const _` block beside the enum:
//!
//! - the enum again for serde to derive as written, `#[serde(remote = …)]`
//!   so that it builds the enum itself: serde's own read, buffer and all;
//! - the enum again without its tag, an externally tagged enum to serde, the
//!   same way: serde's read of each variant by name, every variant and field
//!   attribute as written;
//! - `TypeFirst` naming the two, and `Deserialize` calling the runtime.
//!
//! The variants are copied with their `#[serde]` attributes and nothing else
//! (a doc comment, a `#[schemars]`, a discriminant mean nothing to a read), so
//! every rename, alias, default, `other`, `skip`, `with` and
//! `deny_unknown_fields` is serde's own, read by serde's own code.

use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2, TokenTree};
use proc_macro_crate::{crate_name, FoundCrate};
use quote::quote;
use syn::{Attribute, Data, DeriveInput, Fields, LitStr};

pub(crate) fn derive(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as DeriveInput);
    match expand(&input) {
        Ok(tokens) => tokens.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// Why this derive is not for the type, and what is.
fn refuse(span: Span, why: &str) -> syn::Error {
    syn::Error::new(
        span,
        format!(
            "`plexus_macros::Deserialize` reads an internally tagged enum (`#[serde(tag = \"...\")]`) with its tag \
             first; {why}: derive serde's `Deserialize` for this type"
        ),
    )
}

fn expand(input: &DeriveInput) -> syn::Result<TokenStream2> {
    let ident = &input.ident;
    let Data::Enum(data) = &input.data else {
        return Err(refuse(ident.span(), "this is not an enum"));
    };
    if !input.generics.params.is_empty() || input.generics.where_clause.is_some() {
        return Err(refuse(ident.span(), "a generic enum is not read by it"));
    }

    // The container's `#[serde]` attributes: kept as written for serde's
    // read, and all but the tag for the read by name.
    let mut tag = None;
    let mut expecting = None;
    let mut own_crate = false;
    let mut tagged_attrs: Vec<&Attribute> = Vec::new();
    let mut named_items: Vec<TokenStream2> = Vec::new();
    for attr in serde_attrs(&input.attrs) {
        let list = attr.meta.require_list()?;
        for item in items(list.tokens.clone()) {
            match name(&item).as_deref() {
                Some("tag") => {
                    tag = Some(string(&item)?);
                    continue;
                }
                Some("content") => return Err(refuse(ident.span(), "an adjacently tagged enum is read by serde")),
                Some("untagged") => return Err(refuse(ident.span(), "an untagged enum has no tag to read first")),
                Some("remote" | "from" | "try_from" | "transparent" | "variant_identifier" | "field_identifier") => {
                    return Err(refuse(ident.span(), "this enum is read through another type"))
                }
                Some("crate") => own_crate = true,
                Some("expecting") => expecting = Some(string(&item)?),
                _ => {}
            }
            named_items.push(item);
        }
        tagged_attrs.push(attr);
    }
    let Some(tag) = tag else {
        return Err(refuse(ident.span(), "this enum has no `tag`"));
    };
    let expecting = expecting.unwrap_or_else(|| format!("internally tagged enum {ident}"));

    // The variants, with their `#[serde]` attributes and their fields'.
    let mut variants = Vec::new();
    for variant in &data.variants {
        let attrs = serde_attrs(&variant.attrs).collect::<Vec<_>>();
        for attr in &attrs {
            if items(attr.meta.require_list()?.tokens.clone()).iter().any(|i| name(i).as_deref() == Some("untagged")) {
                return Err(refuse(
                    variant.ident.span(),
                    "an untagged variant is tried on the whole object once the tagged ones fail, which needs serde's \
                     buffer",
                ));
            }
        }
        let variant_ident = &variant.ident;
        let fields = match &variant.fields {
            Fields::Named(fields) => {
                let fields = fields.named.iter().map(|f| {
                    let attrs = serde_attrs(&f.attrs);
                    let (field, ty) = (&f.ident, &f.ty);
                    quote! { #(#attrs)* #field: #ty }
                });
                quote! { { #(#fields),* } }
            }
            Fields::Unnamed(fields) => {
                let fields = fields.unnamed.iter().map(|f| {
                    let attrs = serde_attrs(&f.attrs);
                    let ty = &f.ty;
                    quote! { #(#attrs)* #ty }
                });
                quote! { ( #(#fields),* ) }
            }
            Fields::Unit => quote! {},
        };
        variants.push(quote! { #(#attrs)* #variant_ident #fields });
    }

    let client = client_path(ident.span())?;
    let rt = quote! { #client::__rt };
    let serde = quote! { #rt::serde };
    let serde_crate = (!own_crate).then(|| {
        let path = quote! { #rt::serde }.to_string().replace(' ', "");
        quote! { #[serde(crate = #path)] }
    });
    let remote = ident.to_string();
    let named_attr = (!named_items.is_empty()).then(|| quote! { #[serde(#(#named_items),*)] });

    Ok(quote! {
        #[doc(hidden)]
        #[allow(non_camel_case_types, dead_code, deprecated, unused_qualifications, clippy::all)]
        const _: () = {
            // serde's read of the enum as written, buffer and all.
            #[derive(#serde::Deserialize)]
            #serde_crate
            #[serde(remote = #remote)]
            #(#tagged_attrs)*
            enum __PlexusTagged {
                #(#variants),*
            }

            // serde's read of each variant by name, the tag left out.
            #[derive(#serde::Deserialize)]
            #serde_crate
            #[serde(remote = #remote)]
            #named_attr
            enum __PlexusNamed {
                #(#variants),*
            }

            #[automatically_derived]
            impl<'de> #rt::type_first::TypeFirst<'de> for #ident {
                const TAG: &'static str = #tag;
                const EXPECTING: &'static str = #expecting;

                fn tagged<__D: #serde::Deserializer<'de>>(
                    __deserializer: __D,
                ) -> ::core::result::Result<Self, __D::Error> {
                    __PlexusTagged::deserialize(__deserializer)
                }

                fn named<__D: #serde::Deserializer<'de>>(
                    __deserializer: __D,
                ) -> ::core::result::Result<Self, __D::Error> {
                    __PlexusNamed::deserialize(__deserializer)
                }
            }

            #[automatically_derived]
            impl<'de> #serde::Deserialize<'de> for #ident {
                fn deserialize<__D: #serde::Deserializer<'de>>(
                    __deserializer: __D,
                ) -> ::core::result::Result<Self, __D::Error> {
                    #rt::type_first::deserialize(__deserializer)
                }
            }
        };
    })
}

/// The `#[serde(...)]` attributes among `attrs`.
fn serde_attrs(attrs: &[Attribute]) -> impl Iterator<Item = &Attribute> + '_ {
    attrs.iter().filter(|a| a.path().is_ident("serde"))
}

/// A `#[serde(...)]` attribute's items, split at its top-level commas.
fn items(tokens: TokenStream2) -> Vec<TokenStream2> {
    let mut items = Vec::new();
    let mut item = TokenStream2::new();
    for token in tokens {
        match &token {
            TokenTree::Punct(p) if p.as_char() == ',' => {
                if !item.is_empty() {
                    items.push(std::mem::take(&mut item));
                }
            }
            _ => item.extend([token]),
        }
    }
    if !item.is_empty() {
        items.push(item);
    }
    items
}

/// An item's name: `tag` of `tag = "type"`, `deny_unknown_fields`, `crate`.
fn name(item: &TokenStream2) -> Option<String> {
    match item.clone().into_iter().next() {
        Some(TokenTree::Ident(ident)) => Some(ident.to_string()),
        _ => None,
    }
}

/// The string an item names: `"type"` of `tag = "type"`.
fn string(item: &TokenStream2) -> syn::Result<String> {
    let mut tokens = item.clone().into_iter().skip(1);
    match (tokens.next(), tokens.next(), tokens.next()) {
        (Some(TokenTree::Punct(eq)), Some(value @ TokenTree::Literal(_)), None) if eq.as_char() == '=' => {
            Ok(syn::parse2::<LitStr>(value.into())?.value())
        }
        _ => Err(syn::Error::new_spanned(item, "expected `name = \"...\"`")),
    }
}

/// plexus-client, which holds the runtime, as the deriving crate names it:
/// `crate` inside plexus-client itself, `::plexus_client` in its tests and
/// in every other crate.
fn client_path(span: Span) -> syn::Result<TokenStream2> {
    match crate_name("plexus-client") {
        Ok(FoundCrate::Itself) if std::env::var("CARGO_CRATE_NAME").as_deref() == Ok("plexus_client") => {
            Ok(quote! { crate })
        }
        Ok(FoundCrate::Itself) => Ok(quote! { ::plexus_client }),
        Ok(FoundCrate::Name(name)) => {
            let name = syn::Ident::new(&name, span);
            Ok(quote! { ::#name })
        }
        Err(e) => Err(syn::Error::new(
            span,
            format!(
                "`plexus_macros::Deserialize` reads through plexus-client's runtime, and plexus-client is not a \
                 dependency ({e}): add `plexus-client` to Cargo.toml"
            ),
        )),
    }
}
