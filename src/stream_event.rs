//! StreamEvent derive macro
//!
//! Generates `ActivationStreamItem` implementation for stream event types.
//! Users mark terminal variants with `#[terminal]` attribute.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{parse_macro_input, punctuated::Punctuated, DeriveInput, Expr, ExprLit, Lit, Meta, MetaNameValue, Token};

pub fn derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match derive_impl(input) {
        Ok(tokens) => tokens.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

fn derive_impl(input: DeriveInput) -> syn::Result<TokenStream2> {
    let name = &input.ident;

    // Extract attributes from #[stream_event(...)]
    let (content_type, crate_path) = extract_stream_event_attrs(&input)?;
    let crate_path: syn::Path = syn::parse_str(&crate_path)?;

    // Generate is_terminal() based on #[terminal] attributes on variants
    let terminal_check = generate_terminal_check(&input)?;

    // Z2H-2: resolve serde_json absolutely (umbrella-aware) instead of
    // relying on the consumer's extern prelude.
    let sj = crate::codegen::derive_serde_json_path(proc_macro2::Span::call_site());

    Ok(quote! {
        impl #crate_path::plugin_system::types::ActivationStreamItem for #name {
            fn content_type() -> &'static str
            where
                Self: Sized,
            {
                #content_type
            }

            fn into_plexus_item(
                self,
                provenance: #crate_path::plexus::Provenance,
                plexus_hash: &str,
            ) -> #crate_path::plexus::PlexusStreamItem {
                #crate_path::plexus::PlexusStreamItem::data(
                    plexus_hash.to_string(),
                    provenance,
                    Self::content_type().to_string(),
                    #sj::to_value(&self).unwrap(),
                )
            }

            fn is_terminal(&self) -> bool {
                #terminal_check
            }
        }
    })
}

fn extract_stream_event_attrs(input: &DeriveInput) -> syn::Result<(String, String)> {
    // Defaults
    let mut content_type = input.ident.to_string().to_lowercase();
    let mut crate_path = "crate".to_string();

    for attr in &input.attrs {
        if attr.path().is_ident("stream_event") {
            if let Meta::List(list) = &attr.meta {
                let nested: Punctuated<Meta, Token![,]> =
                    list.parse_args_with(Punctuated::parse_terminated)?;
                for meta in nested {
                    if let Meta::NameValue(MetaNameValue { path, value, .. }) = meta {
                        if let Expr::Lit(ExprLit { lit: Lit::Str(s), .. }) = value {
                            if path.is_ident("content_type") {
                                content_type = s.value();
                            } else if path.is_ident("crate_path") {
                                crate_path = s.value();
                            }
                        }
                    }
                }
            }
        }
    }

    Ok((content_type, crate_path))
}

fn generate_terminal_check(input: &DeriveInput) -> syn::Result<TokenStream2> {
    match &input.data {
        syn::Data::Enum(data_enum) => {
            // Find variants marked with #[terminal]
            let terminal_arms: Vec<TokenStream2> = data_enum
                .variants
                .iter()
                .filter_map(|variant| {
                    let has_terminal = variant.attrs.iter().any(|a| a.path().is_ident("terminal"));
                    if has_terminal {
                        let variant_name = &variant.ident;
                        // Handle different variant types
                        let pattern = match &variant.fields {
                            syn::Fields::Unit => quote! { Self::#variant_name },
                            syn::Fields::Named(_) => quote! { Self::#variant_name { .. } },
                            syn::Fields::Unnamed(_) => quote! { Self::#variant_name(..) },
                        };
                        Some(quote! { #pattern => true, })
                    } else {
                        None
                    }
                })
                .collect();

            if terminal_arms.is_empty() {
                // No terminal variants - always false
                Ok(quote! { false })
            } else {
                Ok(quote! {
                    match self {
                        #(#terminal_arms)*
                        _ => false,
                    }
                })
            }
        }
        syn::Data::Struct(_) => {
            // Structs are single-response, always terminal
            Ok(quote! { true })
        }
        syn::Data::Union(_) => Err(syn::Error::new_spanned(
            input,
            "StreamEvent cannot be derived for unions",
        )),
    }
}
