//! Attribute parsing for hub macros

use std::collections::HashMap;
use syn::{
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
    Expr, ExprLit, FnArg, GenericArgument, ImplItemFn, Lit, Meta, MetaList, MetaNameValue, Pat,
    PathArguments, ReturnType, Token, Type,
};

/// Parsed attributes for #[hub_method]
pub struct HubMethodAttrs {
    pub name: Option<String>,
    /// Parameter descriptions: param_name -> description
    pub param_docs: HashMap<String, String>,
    /// Specific enum variants this method can return (for filtered schema generation)
    /// When specified, the return schema will only include these variants from the event enum.
    /// Format: returns(Variant1, Variant2, Error)
    pub returns_variants: Vec<String>,
    /// If true, this method streams multiple events over time.
    /// If false (default), it yields a single result.
    /// Used to determine client API: streaming -> AsyncGenerator, non-streaming -> Promise
    pub streaming: bool,
    /// If true, this method supports bidirectional communication via BidirChannel.
    /// The method must have a parameter with type containing "BidirChannel".
    pub bidirectional: bool,
}

impl Parse for HubMethodAttrs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut name = None;
        let mut param_docs = HashMap::new();
        let mut returns_variants = Vec::new();
        let mut streaming = false;
        let mut bidirectional = false;

        if !input.is_empty() {
            let metas = Punctuated::<Meta, Token![,]>::parse_terminated(input)?;
            for meta in metas {
                match meta {
                    Meta::Path(path) => {
                        if path.is_ident("override_call") {
                            // Deprecated: use streaming pattern instead
                            // e.g., return impl Stream<Item = MyEvent> and forward PlexusStreamItem
                            // into your event type
                            return Err(syn::Error::new_spanned(
                                &path,
                                "override_call is deprecated. Use the streaming pattern instead:\n\
                                 - Return `impl Stream<Item = YourEvent>` from the method\n\
                                 - Forward PlexusStreamItem variants into your event enum\n\
                                 - Add `streaming` attribute if the method yields multiple events\n\
                                 See plexus.call implementation for an example.",
                            ));
                        } else if path.is_ident("streaming") {
                            streaming = true;
                        } else if path.is_ident("bidirectional") {
                            bidirectional = true;
                        }
                    }
                    Meta::NameValue(MetaNameValue { path, value, .. }) => {
                        if path.is_ident("name") {
                            if let Expr::Lit(ExprLit { lit: Lit::Str(s), .. }) = value {
                                name = Some(s.value());
                            }
                        }
                    }
                    Meta::List(MetaList { path, tokens, .. }) => {
                        if path.is_ident("params") {
                            // Parse params(name = "desc", other = "desc2")
                            let parser = Punctuated::<Meta, Token![,]>::parse_terminated;
                            let nested = syn::parse::Parser::parse2(parser, tokens.clone())?;
                            for meta in nested {
                                if let Meta::NameValue(MetaNameValue { path, value, .. }) = meta {
                                    if let Some(ident) = path.get_ident() {
                                        if let Expr::Lit(ExprLit { lit: Lit::Str(s), .. }) = value {
                                            param_docs.insert(ident.to_string(), s.value());
                                        }
                                    }
                                }
                            }
                        } else if path.is_ident("returns") {
                            // Parse returns(Variant1, Variant2, Error)
                            let parser = Punctuated::<syn::Ident, Token![,]>::parse_terminated;
                            let nested = syn::parse::Parser::parse2(parser, tokens.clone())?;
                            for ident in nested {
                                returns_variants.push(ident.to_string());
                            }
                        }
                    }
                }
            }
        }

        Ok(HubMethodAttrs { name, param_docs, returns_variants, streaming, bidirectional })
    }
}

/// Maximum word count for description
const MAX_DESCRIPTION_WORDS: usize = 15;

/// Parsed attributes for #[hub_methods]
pub struct HubMethodsAttrs {
    pub namespace: String,
    pub version: String,
    /// Short description (max 15 words)
    pub description: Option<String>,
    /// Long description (optional, for detailed documentation)
    pub long_description: Option<String>,
    pub crate_path: String,
    /// If true, generate resolve_handle that delegates to self.resolve_handle_impl()
    pub resolve_handle: bool,
    /// If true, this activation is a hub with children (calls self.plugin_children())
    pub hub: bool,
    /// Stable UUID for this plugin instance (for handle routing)
    /// If not provided, a deterministic UUID is generated from namespace
    pub plugin_id: Option<String>,
    /// If set, namespace() will call this method instead of returning the constant
    /// e.g., namespace_fn = "runtime_namespace" generates: fn namespace(&self) -> &str { self.runtime_namespace() }
    pub namespace_fn: Option<String>,
}

impl Parse for HubMethodsAttrs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut namespace = String::new();
        let mut version = "1.0.0".to_string();
        let mut description = None;
        let mut long_description = None;
        let mut crate_path = "crate".to_string();
        let mut resolve_handle = false;
        let mut hub = false;
        let mut plugin_id = None;
        let mut namespace_fn = None;

        if !input.is_empty() {
            let metas = Punctuated::<Meta, Token![,]>::parse_terminated(input)?;

            for meta in metas {
                match meta {
                    Meta::NameValue(MetaNameValue { path, value, .. }) => {
                        if let Expr::Lit(ExprLit { lit: Lit::Str(s), .. }) = value {
                            if path.is_ident("namespace") {
                                namespace = s.value();
                            } else if path.is_ident("version") {
                                version = s.value();
                            } else if path.is_ident("description") {
                                let desc = s.value();
                                // Validate word count
                                let word_count = desc.split_whitespace().count();
                                if word_count > MAX_DESCRIPTION_WORDS {
                                    return Err(syn::Error::new(
                                        s.span(),
                                        format!(
                                            "description must be {} words or fewer (found {} words). \
                                            Use long_description for detailed text.",
                                            MAX_DESCRIPTION_WORDS, word_count
                                        ),
                                    ));
                                }
                                description = Some(desc);
                            } else if path.is_ident("long_description") {
                                long_description = Some(s.value());
                            } else if path.is_ident("crate_path") {
                                crate_path = s.value();
                            } else if path.is_ident("plugin_id") {
                                // Validate UUID format
                                let id_str = s.value();
                                if uuid::Uuid::parse_str(&id_str).is_err() {
                                    return Err(syn::Error::new(
                                        s.span(),
                                        format!("plugin_id must be a valid UUID, got: {}", id_str),
                                    ));
                                }
                                plugin_id = Some(id_str);
                            } else if path.is_ident("namespace_fn") {
                                namespace_fn = Some(s.value());
                            }
                        }
                    }
                    Meta::Path(path) => {
                        // Handle bare flags like `resolve_handle`, `hub`
                        if path.is_ident("resolve_handle") {
                            resolve_handle = true;
                        } else if path.is_ident("hub") {
                            hub = true;
                        }
                    }
                    _ => {}
                }
            }
        }

        if namespace.is_empty() {
            return Err(syn::Error::new(
                input.span(),
                "hub_methods requires namespace = \"...\" attribute",
            ));
        }

        Ok(HubMethodsAttrs { namespace, version, description, long_description, crate_path, resolve_handle, hub, plugin_id, namespace_fn })
    }
}

/// Parameter info with optional description
pub struct ParamInfo {
    pub name: syn::Ident,
    pub ty: Type,
    pub description: Option<String>,
}

/// Information extracted from a #[hub_method] function
pub struct MethodInfo {
    pub fn_name: syn::Ident,
    pub method_name: String,
    pub description: String,
    pub params: Vec<ParamInfo>,
    pub return_type: Type,
    pub stream_item_type: Option<Type>,
    /// Specific enum variants this method can return (empty = all variants)
    pub returns_variants: Vec<String>,
    /// True if method streams multiple events (from #[hub_method(streaming)])
    /// False (default) means method yields a single result
    pub streaming: bool,
    /// True if method supports bidirectional communication via BidirChannel
    pub bidirectional: bool,
    /// Name of the BidirChannel parameter (if bidirectional is true)
    pub context_param: Option<syn::Ident>,
}

impl MethodInfo {
    /// Extract method info from an impl function with optional hub_method attrs
    pub fn from_fn(method: &ImplItemFn, hub_method_attrs: Option<&HubMethodAttrs>) -> syn::Result<Self> {
        let fn_name = method.sig.ident.clone();
        let method_name = hub_method_attrs
            .and_then(|a| a.name.clone())
            .unwrap_or_else(|| fn_name.to_string());

        // Extract doc attributes and description
        let mut doc_lines = Vec::new();
        for attr in &method.attrs {
            if attr.path().is_ident("doc") {
                if let Meta::NameValue(MetaNameValue { value, .. }) = &attr.meta {
                    if let Expr::Lit(ExprLit { lit: Lit::Str(s), .. }) = value {
                        doc_lines.push(s.value().trim().to_string());
                    }
                }
            }
        }
        let description = doc_lines.join(" ");

        // Get param docs and returns_variants from hub_method attrs
        let param_docs = hub_method_attrs
            .map(|a| &a.param_docs)
            .cloned()
            .unwrap_or_default();
        let returns_variants = hub_method_attrs
            .map(|a| a.returns_variants.clone())
            .unwrap_or_default();
        let streaming = hub_method_attrs
            .map(|a| a.streaming)
            .unwrap_or(false);
        let bidirectional = hub_method_attrs
            .map(|a| a.bidirectional)
            .unwrap_or(false);

        // Extract parameters after &self
        // Also track the BidirChannel parameter if bidirectional is true
        let mut params = Vec::new();
        let mut context_param: Option<syn::Ident> = None;
        for arg in &method.sig.inputs {
            if let FnArg::Typed(pat_type) = arg {
                if let Pat::Ident(ident) = &*pat_type.pat {
                    let name = ident.ident.clone();
                    let name_str = name.to_string();

                    // Check if this parameter's type contains "BidirChannel"
                    if bidirectional && contains_bidir_channel(&pat_type.ty) {
                        context_param = Some(name.clone());
                        // Skip adding BidirChannel to params - it's not a client-facing parameter
                        continue;
                    }

                    let description = param_docs.get(&name_str).cloned();
                    params.push(ParamInfo {
                        name,
                        ty: (*pat_type.ty).clone(),
                        description,
                    });
                }
            }
        }

        // Extract return type
        let return_type = match &method.sig.output {
            ReturnType::Default => {
                return Err(syn::Error::new_spanned(
                    &method.sig,
                    "hub_method requires a return type",
                ))
            }
            ReturnType::Type(_, ty) => (**ty).clone(),
        };

        let stream_item_type = extract_stream_item_type(&return_type);

        // Check for PlexusStream return type - must use streaming pattern instead
        if is_result_plexus_stream(&return_type) {
            return Err(syn::Error::new_spanned(
                &method.sig.output,
                format!(
                    "Method `{}` returns Result<PlexusStream, _> which bypasses schema generation. \
                    Use the streaming pattern instead:\n\
                    - Return `impl Stream<Item = YourEvent>` from the method\n\
                    - Forward PlexusStreamItem variants into your event enum\n\
                    - Add `streaming` attribute if the method yields multiple events\n\
                    See plexus.call implementation for an example.",
                    fn_name
                ),
            ));
        }

        Ok(MethodInfo {
            fn_name,
            method_name,
            description,
            params,
            return_type,
            stream_item_type,
            returns_variants,
            streaming,
            bidirectional,
            context_param,
        })
    }
}

/// Extract Item type from `impl Stream<Item = T>`
fn extract_stream_item_type(ty: &Type) -> Option<Type> {
    if let Type::ImplTrait(impl_trait) = ty {
        for bound in &impl_trait.bounds {
            if let syn::TypeParamBound::Trait(trait_bound) = bound {
                let last_segment = trait_bound.path.segments.last()?;
                if last_segment.ident == "Stream" {
                    if let PathArguments::AngleBracketed(args) = &last_segment.arguments {
                        for arg in &args.args {
                            if let GenericArgument::AssocType(assoc) = arg {
                                if assoc.ident == "Item" {
                                    return Some(assoc.ty.clone());
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

/// Check if return type looks like Result<PlexusStream, _>
/// This is now rejected - methods should use the streaming pattern instead
fn is_result_plexus_stream(ty: &Type) -> bool {
    if let Type::Path(type_path) = ty {
        if let Some(segment) = type_path.path.segments.last() {
            if segment.ident == "Result" {
                if let PathArguments::AngleBracketed(args) = &segment.arguments {
                    if let Some(GenericArgument::Type(first_type)) = args.args.first() {
                        // Check if first type arg is PlexusStream
                        if let Type::Path(inner_path) = first_type {
                            if let Some(inner_seg) = inner_path.path.segments.last() {
                                return inner_seg.ident == "PlexusStream";
                            }
                        }
                    }
                }
            }
        }
    }
    false
}

/// Check if a type contains "BidirChannel" in its path
/// This handles types like `BidirChannel<T>`, `&BidirChannel<T>`, etc.
fn contains_bidir_channel(ty: &Type) -> bool {
    match ty {
        Type::Path(type_path) => {
            // Check if any segment is "BidirChannel"
            type_path.path.segments.iter().any(|seg| seg.ident == "BidirChannel")
        }
        Type::Reference(type_ref) => {
            // Check inside references like &BidirChannel or &mut BidirChannel
            contains_bidir_channel(&type_ref.elem)
        }
        _ => false,
    }
}
