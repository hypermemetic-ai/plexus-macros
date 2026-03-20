//! Generate method enum from hub methods

use crate::parse::{BidirType, MethodInfo};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

pub fn generate(struct_name: &syn::Ident, methods: &[MethodInfo], crate_path: &syn::Path) -> TokenStream {
    let enum_name = format_ident!("{}Method", struct_name);

    let variants: Vec<TokenStream> = methods
        .iter()
        .map(|m| {
            let variant_name = format_ident!("{}", to_pascal_case(&m.method_name));
            let method_name = &m.method_name;
            let doc = &m.description;

            // Always use struct variants for consistent schema generation.
            // This ensures params is always an object with named fields.
            // Use serde(rename) to get the exact method name in the schema.
            match m.params.len() {
                0 => quote! {
                    #[doc = #doc]
                    #[serde(rename = #method_name)]
                    #variant_name
                },
                _ => {
                    // Use struct variant for all cases (including single param)
                    // This produces proper object schema with field names
                    // Add description for each field from param_docs or fallback to name
                    // Add #[serde(default, deserialize_with = "...")] for Option<T> fields
                    // to make them truly optional and accept explicit null values
                    let fields: Vec<TokenStream> = m
                        .params
                        .iter()
                        .map(|p| {
                            let name = &p.name;
                            let ty = &p.ty;
                            let desc = p.description.clone()
                                .unwrap_or_else(|| format!("The {} parameter", name));

                            // Check if type is Option<T> - if so, add serde attributes
                            // that handle both missing fields AND explicit null values
                            let is_option = is_option_type(ty);
                            if is_option {
                                // Always use crate::serde_helpers since consuming crates should
                                // re-export plexus_core::serde_helpers (or define their own)
                                quote! {
                                    #[schemars(description = #desc)]
                                    #[serde(default, deserialize_with = "crate::serde_helpers::deserialize_null_as_none")]
                                    #name: #ty
                                }
                            } else {
                                quote! {
                                    #[schemars(description = #desc)]
                                    #name: #ty
                                }
                            }
                        })
                        .collect();
                    quote! {
                        #[doc = #doc]
                        #[serde(rename = #method_name)]
                        #variant_name { #(#fields),* }
                    }
                }
            }
        })
        .collect();

    let method_names: Vec<&str> = methods.iter().map(|m| m.method_name.as_str()).collect();
    let method_descriptions: Vec<&str> = methods.iter().map(|m| m.description.as_str()).collect();

    // Compute hashes at compile time for each method
    let method_hashes: Vec<String> = methods.iter().map(compute_method_hash).collect();

    // Generate return type schemas for each method
    // Each entry is a tuple of (full_schema, variant_filter)
    // If variant_filter is non-empty, we filter the oneOf to only those variants
    let return_schema_entries: Vec<TokenStream> = methods
        .iter()
        .map(|m| {
            if let Some(item_ty) = &m.stream_item_type {
                if m.returns_variants.is_empty() {
                    // No filtering - return full schema
                    quote! { (Some(schemars::schema_for!(#item_ty)), Vec::<&str>::new()) }
                } else {
                    // Return schema with filter list
                    let variants = &m.returns_variants;
                    quote! { (Some(schemars::schema_for!(#item_ty)), vec![#(#variants),*]) }
                }
            } else {
                quote! { (None, Vec::<&str>::new()) }
            }
        })
        .collect();

    // Generate streaming flags for each method
    // Streaming is explicitly declared via #[hub_method(streaming)] attribute
    let streaming_flags: Vec<bool> = methods
        .iter()
        .map(|m| m.streaming)
        .collect();

    // Generate HTTP method enum values for each method (defaults to POST if not specified)
    let http_methods: Vec<TokenStream> = methods
        .iter()
        .map(|m| {
            match m.http_method.as_deref() {
                Some("GET") => quote! { #crate_path::plexus::schema::HttpMethod::Get },
                Some("POST") => quote! { #crate_path::plexus::schema::HttpMethod::Post },
                Some("PUT") => quote! { #crate_path::plexus::schema::HttpMethod::Put },
                Some("DELETE") => quote! { #crate_path::plexus::schema::HttpMethod::Delete },
                Some("PATCH") => quote! { #crate_path::plexus::schema::HttpMethod::Patch },
                None => quote! { #crate_path::plexus::schema::HttpMethod::Post }, // Default
                _ => quote! { #crate_path::plexus::schema::HttpMethod::Post }, // Fallback (should not happen due to validation)
            }
        })
        .collect();

    // Generate bidirectional schema setup calls and their method indices.
    //
    // Each entry is a (index, TokenStream) pair where the TokenStream fragment
    // applies the appropriate `.with_*` builder calls to configure the
    // MethodSchema for bidirectional communication.
    //
    // - BidirType::None     → empty fragment (no-op; skipped via index filter)
    // - BidirType::Standard → `schema = schema.with_standard_bidirectional();`
    // - BidirType::Custom   → `schema = schema.with_bidirectional(true)
    //                           .with_request_type(schemars::schema_for!(Req).into())
    //                           .with_response_type(schemars::schema_for!(Resp).into());`
    //
    // Both `bidir_index` and `bidir_schema_calls` are zipped together in the
    // generated `match i { #bidir_index => { #bidir_schema_calls } }` block.
    let (bidir_index, bidir_schema_calls): (Vec<_>, Vec<_>) = methods
        .iter()
        .enumerate()
        .filter_map(|(idx, m)| {
            let idx_lit = proc_macro2::Literal::usize_suffixed(idx);
            match &m.bidirectional {
                BidirType::None => None,
                BidirType::Standard => Some((
                    quote! { #idx_lit },
                    quote! { schema = schema.with_standard_bidirectional(); },
                )),
                BidirType::Custom { request, response } => {
                    let req_ty: syn::Type = syn::parse_str(request)
                        .unwrap_or_else(|_| syn::parse_str("serde_json::Value").unwrap());
                    let resp_ty: syn::Type = syn::parse_str(response)
                        .unwrap_or_else(|_| syn::parse_str("serde_json::Value").unwrap());
                    Some((
                        quote! { #idx_lit },
                        quote! {
                            schema = schema
                                .with_bidirectional(true)
                                .with_request_type(schemars::schema_for!(#req_ty).into())
                                .with_response_type(schemars::schema_for!(#resp_ty).into());
                        },
                    ))
                }
            }
        })
        .unzip();

    quote! {
        /// Auto-generated method enum for schema extraction
        #[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
        #[serde(tag = "method", content = "params", rename_all = "snake_case")]
        pub enum #enum_name {
            #(#variants),*
        }

        impl #enum_name {
            pub fn all_method_names() -> &'static [&'static str] {
                &[#(#method_names),*]
            }

            /// Cached schema value - computed once on first access
            fn cached_schema() -> &'static serde_json::Value {
                static SCHEMA_CACHE: std::sync::OnceLock<serde_json::Value> = std::sync::OnceLock::new();
                SCHEMA_CACHE.get_or_init(|| {
                    serde_json::to_value(schemars::schema_for!(#enum_name)).expect("Schema should serialize")
                })
            }
        }

        impl #crate_path::plexus::MethodEnumSchema for #enum_name {
            fn method_names() -> &'static [&'static str] {
                &[#(#method_names),*]
            }

            fn schema_with_consts() -> serde_json::Value {
                // Return cached schema (cloned since caller may mutate)
                Self::cached_schema().clone()
            }
        }

        impl #enum_name {
            /// Get per-method schema info including params, return types, and content hashes
            ///
            /// Note: This method has O(1) schema lookup cost after first call due to caching.
            pub fn method_schemas() -> Vec<#crate_path::plexus::MethodSchema> {
                // Cache the computed method schemas for O(1) subsequent calls
                static METHOD_SCHEMAS_CACHE: std::sync::OnceLock<Vec<#crate_path::plexus::MethodSchema>> = std::sync::OnceLock::new();

                METHOD_SCHEMAS_CACHE.get_or_init(|| {
                    Self::compute_method_schemas()
                }).clone()
            }

            /// Internal: compute method schemas (called once, then cached)
            fn compute_method_schemas() -> Vec<#crate_path::plexus::MethodSchema> {
                let method_names: &[&str] = &[#(#method_names),*];
                let descriptions: &[&str] = &[#(#method_descriptions),*];
                let hashes: &[&str] = &[#(#method_hashes),*];
                let streaming: &[bool] = &[#(#streaming_flags),*];
                let http_methods: Vec<#crate_path::plexus::schema::HttpMethod> = vec![#(#http_methods),*];
                let return_schemas: Vec<(Option<schemars::Schema>, Vec<&str>)> = vec![#(#return_schema_entries),*];

                // Get the cached full enum schema
                let schema_value = Self::cached_schema();

                // Extract $defs from the root schema - these need to be merged into each method's params
                // because types like ConeIdentifier are defined at the root level but referenced via $ref
                let root_defs = schema_value.get("$defs").cloned();

                // Extract oneOf variants from the schema
                let one_of = schema_value
                    .get("oneOf")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();

                let mut methods: Vec<_> = method_names
                    .iter()
                    .zip(descriptions.iter())
                    .zip(hashes.iter())
                    .zip(streaming.iter())
                    .zip(http_methods.into_iter())
                    .zip(return_schemas.into_iter())
                    .enumerate()
                    .map(|(i, (((((name, desc), hash), is_streaming), http_method), (returns_opt, variant_filter)))| {
                        // Get this variant's schema from oneOf, then extract just the "params" portion
                        // The variant looks like: { properties: { method: {...}, params: {...} }, ... }
                        // We want just the params schema, but we need to merge in $defs from the root
                        let params = one_of.get(i).and_then(|variant| {
                            variant
                                .get("properties")
                                .and_then(|props| props.get("params"))
                                .cloned()
                                .and_then(|mut p| {
                                    // Merge root $defs into the params schema so $ref references resolve
                                    if let (Some(params_obj), Some(defs)) = (p.as_object_mut(), &root_defs) {
                                        params_obj.insert("$defs".to_string(), defs.clone());
                                    }
                                    serde_json::from_value::<schemars::Schema>(p).ok()
                                })
                        });

                        // Filter return schema if variant_filter is specified
                        let filtered_returns = returns_opt.map(|schema| {
                            if variant_filter.is_empty() {
                                schema
                            } else {
                                Self::filter_return_schema(schema, &variant_filter)
                            }
                        });

                        let mut schema = #crate_path::plexus::MethodSchema::new(
                            name.to_string(),
                            desc.to_string(),
                            hash.to_string(),
                        );
                        if let Some(p) = params {
                            schema = schema.with_params(p);
                        }
                        if let Some(r) = filtered_returns {
                            schema = schema.with_returns(r);
                        }
                        schema = schema.with_streaming(*is_streaming);
                        schema = schema.with_http_method(http_method);

                        // Apply bidirectional schema configuration.
                        // Uses a compile-time match on the method index so that type-level
                        // calls like schemars::schema_for!(MyType) work correctly.
                        // Each arm is generated at macro-expansion time.
                        match i {
                            #(
                                #bidir_index => {
                                    #bidir_schema_calls
                                }
                            )*
                            _ => {}
                        }

                        schema
                    })
                    .collect::<Vec<_>>();

                // Add the auto-generated schema method
                let schema_method = #crate_path::plexus::MethodSchema::new(
                    "schema".to_string(),
                    "Get plugin or method schema. Pass {\"method\": \"name\"} for a specific method.".to_string(),
                    "auto_schema".to_string(), // Fixed hash since it's auto-generated
                );
                methods.push(schema_method);

                methods
            }

            /// Filter a return schema to only include specified variants
            ///
            /// This handles the case where a method returns an enum but only uses
            /// specific variants. The schema is filtered at runtime to only include
            /// those variants in the oneOf array.
            fn filter_return_schema(schema: schemars::Schema, allowed_variants: &[&str]) -> schemars::Schema {
                // Convert to JSON for manipulation
                let mut schema_value = serde_json::to_value(&schema).expect("Schema should serialize");

                // Check if this is a oneOf enum schema
                if let Some(one_of) = schema_value.get_mut("oneOf").and_then(|v| v.as_array_mut()) {
                    // Filter to only variants whose "type" field (the discriminant) matches allowed_variants
                    // serde's internally tagged enums produce: { "type": "variant_name", ...fields }
                    // We need to check the "const" value in the "type" property
                    one_of.retain(|variant| {
                        // Try to find the variant's tag name
                        let variant_name = variant
                            .get("properties")
                            .and_then(|props| props.get("type"))
                            .and_then(|type_prop| type_prop.get("const"))
                            .and_then(|c| c.as_str())
                            .or_else(|| {
                                // Some schemas use "enum" instead of "const"
                                variant
                                    .get("properties")
                                    .and_then(|props| props.get("type"))
                                    .and_then(|type_prop| type_prop.get("enum"))
                                    .and_then(|e| e.as_array())
                                    .and_then(|arr| arr.first())
                                    .and_then(|v| v.as_str())
                            });

                        if let Some(name) = variant_name {
                            // Convert snake_case variant name to PascalCase for comparison
                            let pascal_name = name.split('_')
                                .map(|word| {
                                    let mut chars = word.chars();
                                    match chars.next() {
                                        None => String::new(),
                                        Some(first) => first.to_uppercase().chain(chars).collect(),
                                    }
                                })
                                .collect::<String>();

                            allowed_variants.contains(&pascal_name.as_str()) ||
                            allowed_variants.contains(&name)
                        } else {
                            // Can't determine variant name, keep it
                            true
                        }
                    });
                }

                // Convert back to Schema
                serde_json::from_value(schema_value).expect("Filtered schema should deserialize")
            }
        }
    }
}

fn to_pascal_case(s: &str) -> String {
    s.split('_')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().chain(chars).collect(),
            }
        })
        .collect()
}

/// Compute a hash for a method definition
///
/// The hash is computed from:
/// - Method name
/// - Parameter names and types (stringified)
/// - Description
///
/// This provides cache invalidation at the method level -
/// if any aspect of the method signature changes, the hash changes.
fn compute_method_hash(method: &MethodInfo) -> String {
    let mut hasher = DefaultHasher::new();

    // Hash method name
    method.method_name.hash(&mut hasher);

    // Hash description
    method.description.hash(&mut hasher);

    // Hash each parameter (name + type as string)
    for param in &method.params {
        param.name.to_string().hash(&mut hasher);
        // Convert type to string for hashing
        let ty = &param.ty;
        let ty_str = quote!(#ty).to_string();
        ty_str.hash(&mut hasher);
        if let Some(desc) = &param.description {
            desc.hash(&mut hasher);
        }
    }

    // Hash return type if present
    if let Some(item_ty) = &method.stream_item_type {
        let ty_str = quote!(#item_ty).to_string();
        ty_str.hash(&mut hasher);
    }

    format!("{:016x}", hasher.finish())
}

/// Check if a type is Option<T>
///
/// This is used to determine if a field should have #[serde(default)]
/// so that it can be omitted from the JSON input.
pub fn is_option_type(ty: &syn::Type) -> bool {
    if let syn::Type::Path(type_path) = ty {
        if let Some(segment) = type_path.path.segments.last() {
            return segment.ident == "Option";
        }
    }
    false
}
