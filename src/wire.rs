//! PLEXUS-59 · `#[derive(PlexusWire)]`: serde's `Serialize` and `Deserialize`
//! and schemars' `JsonSchema` from one derive.
//!
//! A derive cannot add a derive to the type it is on, so it writes a twin of
//! the type — the same fields, the type's own `#[serde]`, `#[schemars]` and
//! docs — that derives the three as a serde *remote* of the type, and
//! implements each trait for the type through the twin. The twin lives in an
//! unnamed `const`, beside the type, so it reads private fields and is seen
//! by nothing.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Attribute, Data, DeriveInput, Fields};

fn kept(a: &Attribute) -> bool {
    ["serde", "schemars", "doc", "cfg"].iter().any(|n| a.path().is_ident(n))
}

fn keep_fields(fields: &mut Fields) {
    for f in fields.iter_mut() {
        f.attrs.retain(kept);
    }
}

/// Whether the type's own `#[serde(…)]` says one of `words`.
fn says(attrs: &[Attribute], words: &[&str]) -> bool {
    attrs.iter().filter(|a| a.path().is_ident("serde")).any(|a| {
        let mut said = false;
        let _ = a.parse_nested_meta(|m| {
            said |= words.iter().any(|w| m.path.is_ident(w));
            if m.input.peek(syn::Token![=]) {
                let _: syn::Expr = m.value()?.parse()?;
            }
            Ok(())
        });
        said
    })
}

pub fn derive(input: DeriveInput) -> syn::Result<TokenStream> {
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.generics,
            "PlexusWire is for a type with no generics: derive serde's `Serialize` and `Deserialize` and schemars' `JsonSchema` on this one",
        ));
    }
    let name = &input.ident;
    let name_str = name.to_string();
    let twin = format_ident!("__{}PlexusWire", name);

    let mut def = input.clone();
    def.ident = twin.clone();
    def.vis = syn::Visibility::Inherited;
    def.attrs.retain(kept);
    let is_enum = match &mut def.data {
        Data::Struct(s) => {
            keep_fields(&mut s.fields);
            false
        }
        Data::Enum(e) => {
            for v in e.variants.iter_mut() {
                v.attrs.retain(kept);
                keep_fields(&mut v.fields);
            }
            true
        }
        Data::Union(u) => {
            return Err(syn::Error::new_spanned(u.union_token, "PlexusWire is for a struct or an enum"));
        }
    };

    let (serde, schemars, crates) = match crate::codegen::umbrella_crate(proc_macro2::Span::call_site()) {
        Some(u) => {
            let (s, j) = (format!("::{u}::__deps::serde"), format!("::{u}::__deps::schemars"));
            (quote! { ::#u::__deps::serde }, quote! { ::#u::__deps::schemars }, quote! { #[serde(crate = #s)] #[schemars(crate = #j)] })
        }
        None => (quote! { ::serde }, quote! { ::schemars }, quote! {}),
    };
    // An enum that names its own representation keeps it; a type that names itself keeps its name.
    let tag = if is_enum && !says(&input.attrs, &["tag", "untagged"]) {
        quote! { #[serde(tag = "type", rename_all = "snake_case")] }
    } else {
        quote! {}
    };
    let rename = if says(&input.attrs, &["rename"]) { quote! {} } else { quote! { #[serde(rename = #name_str)] } };

    Ok(quote! {
        const _: () = {
            #[derive(#serde::Serialize, #serde::Deserialize, #schemars::JsonSchema)]
            #crates
            #[serde(remote = #name_str)]
            #rename
            #tag
            #[allow(non_camel_case_types, dead_code)]
            #def

            impl #serde::Serialize for #name {
                fn serialize<S: #serde::Serializer>(&self, serializer: S) -> ::core::result::Result<S::Ok, S::Error> {
                    #twin::serialize(self, serializer)
                }
            }
            impl<'de> #serde::Deserialize<'de> for #name {
                fn deserialize<D: #serde::Deserializer<'de>>(deserializer: D) -> ::core::result::Result<Self, D::Error> {
                    #twin::deserialize(deserializer)
                }
            }
            impl #schemars::JsonSchema for #name {
                fn schema_name() -> ::std::borrow::Cow<'static, str> {
                    ::std::borrow::Cow::Borrowed(#name_str)
                }
                fn schema_id() -> ::std::borrow::Cow<'static, str> {
                    ::std::borrow::Cow::Borrowed(::core::concat!(::core::module_path!(), "::", #name_str))
                }
                fn json_schema(generator: &mut #schemars::SchemaGenerator) -> #schemars::Schema {
                    <#twin as #schemars::JsonSchema>::json_schema(generator)
                }
                fn inline_schema() -> bool {
                    <#twin as #schemars::JsonSchema>::inline_schema()
                }
            }
        };
    })
}
