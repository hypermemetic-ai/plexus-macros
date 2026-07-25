//! Generate method enum from hub methods

use crate::parse::{AuthPosture, BidirType, ChildMethodInfo, ChildMethodKind, MethodInfo, MethodRequestOverride, ParsedDeprecation};
use proc_macro2::TokenStream;
use quote::{format_ident, quote, ToTokens};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use syn::{GenericArgument, PathArguments, Type};

/// PLX-71: reconstruct a parsed `serde_json::Value` extension value into tokens
/// that rebuild the same value at runtime, resolving `serde_json` through the
/// per-expansion dependency shim (`deps`). The `#[method(ext(...))]` parser
/// only produces string / integer / bool values; other JSON shapes fall back to
/// a general `from_str` reconstruction so nothing is silently dropped.
fn ext_value_tokens(value: &serde_json::Value, deps: &syn::Ident) -> TokenStream {
    match value {
        serde_json::Value::String(s) => {
            quote! { #deps::serde_json::Value::String(#s.to_string()) }
        }
        serde_json::Value::Bool(b) => {
            quote! { #deps::serde_json::Value::Bool(#b) }
        }
        serde_json::Value::Number(n) => {
            if let Some(u) = n.as_u64() {
                let lit = proc_macro2::Literal::u64_unsuffixed(u);
                quote! { #deps::serde_json::json!(#lit) }
            } else if let Some(i) = n.as_i64() {
                let lit = proc_macro2::Literal::i64_unsuffixed(i);
                quote! { #deps::serde_json::json!(#lit) }
            } else {
                let f = n.as_f64().unwrap_or(0.0);
                let lit = proc_macro2::Literal::f64_unsuffixed(f);
                quote! { #deps::serde_json::json!(#lit) }
            }
        }
        other => {
            // Forward-compat safety net for any non-literal value the parser
            // might accept in future: rebuild it from its JSON text.
            let json = other.to_string();
            quote! {
                #deps::serde_json::from_str::<#deps::serde_json::Value>(#json)
                    .expect("PLX-71: extension value must be valid JSON")
            }
        }
    }
}

pub fn generate(
    struct_name: &syn::Ident,
    methods: &[MethodInfo],
    child_methods: &[ChildMethodInfo],
    crate_path: &syn::Path,
    auth_core_path: Option<&syn::Path>,
    activation_request_type: Option<&syn::Type>,
    // Z2H-2: ident of the per-expansion dependency shim module. serde /
    // serde_json / schemars references in the emitted enum + schema code
    // resolve relatively through it.
    deps: &syn::Ident,
    // R-1 (joint AP-1): the activation's DECLARED auth posture. `Some(p)` is
    // stamped onto every generated RPC `MethodSchema` via
    // `with_auth_posture`; `None` (posture never declared) emits nothing so
    // posture-silent activations keep their pre-R-1 wire shape.
    auth_posture: Option<AuthPosture>,
) -> TokenStream {
    let enum_name = format_ident!("{}Method", struct_name);

    // Z2H-2: derives are the one place the relative shim path does NOT work —
    // serde/schemars derive expansions emit `use <crate-attr-path> ...` inside
    // their own const blocks, which resolves module-relatively and cannot see
    // a shim module that lives in a fn body (activations declared inside test
    // fns are common). So the derive list + both crates' `crate =` escape
    // hatches use the ABSOLUTE umbrella path in umbrella mode, and today's
    // bare extern-prelude names (with no `crate =` attrs at all) in direct
    // mode — preserving pre-Z2H-2 derive semantics exactly for existing
    // consumers.
    let umbrella = crate::codegen::umbrella_crate(proc_macro2::Span::call_site());
    let (serde_derive_path, schemars_derive_path, derive_crate_attrs) = match &umbrella {
        Some(u) => {
            let serde_crate_attr = format!("::{}::__deps::serde", u);
            let schemars_crate_attr = format!("::{}::__deps::schemars", u);
            (
                quote! { ::#u::__deps::serde },
                quote! { ::#u::__deps::schemars },
                quote! {
                    #[serde(crate = #serde_crate_attr)]
                    #[schemars(crate = #schemars_crate_attr)]
                },
            )
        }
        None => (quote! { serde }, quote! { schemars }, quote! {}),
    };

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
                                let serde_helpers_path = format!("{}::serde_helpers::deserialize_null_as_none",
                                    crate_path.segments.iter().map(|s| s.ident.to_string()).collect::<Vec<_>>().join("::"));
                                quote! {
                                    #[schemars(description = #desc)]
                                    #[serde(default, deserialize_with = #serde_helpers_path)]
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
                    quote! { (Some(#deps::schemars::schema_for!(#item_ty)), Vec::<&str>::new()) }
                } else {
                    // Return schema with filter list
                    let variants = &m.returns_variants;
                    quote! { (Some(#deps::schemars::schema_for!(#item_ty)), vec![#(#variants),*]) }
                }
            } else {
                quote! { (None, Vec::<&str>::new()) }
            }
        })
        .collect();

    // AUTHZ-CRED-MACRO-3: per-method credential-decl-vector expression.
    //
    // For each method, derive the "credential probe type" (the inner success
    // / yielded type of the return signature), then emit a call site that
    // walks the macro-emitted marker registry through the autoref-specialized
    // `CredentialsRegistryProbe`. Types that derive `#[derive(Credentials)]`
    // produce a populated `Vec<CredentialFieldDecl>`; types that don't,
    // produce an empty vec via the trait-default fallback.
    //
    // When `auth_core_path` is `None`, the consuming crate does not depend on
    // `plexus-auth-core`, so we skip emitting the call site — the runtime
    // match falls through to the `_ => Vec::new()` arm and the schema's
    // `credentials` field is unchanged (pre-ticket behavior preserved).
    let (credential_decls_indices, credential_decls_exprs): (Vec<_>, Vec<_>) = methods
        .iter()
        .enumerate()
        .filter_map(|(idx, m)| {
            let auth_path = auth_core_path?;
            let probe_ty = effective_credential_probe_type(m)?;
            let idx_lit = proc_macro2::Literal::usize_suffixed(idx);
            let method_name = &m.method_name;
            // Build the issuer placeholder. At schema-build time we don't
            // know the runtime origin URL, so we synthesize origin "plexus"
            // plus the method's name as the MethodPath. The runtime can
            // re-stamp these at mint time; for schema introspection (IR,
            // CLI, codegen) the issuer is descriptive, not authoritative.
            //
            // `MethodPath::try_new` enforces "namespace.method" grammar
            // (>= 2 dot-separated segments). The method name alone often
            // fails that grammar (e.g. "echo" has no dot), so we fall back
            // to "plexus.<name>" — never panics, always at least 2
            // segments. Activation-level namespace prefixing is a follow-up
            // (see RUN-NOTES "schema issuer fidelity").
            let expr = quote! {
                {
                    // Bring the `CredentialsRegistryFallback` trait into
                    // scope so its empty-slice `marker_slice` resolves via
                    // autoref when the return type does NOT implement
                    // `HasCredentialMarkers`. For return types that DO
                    // implement `HasCredentialMarkers` (via the macro-emitted
                    // `impl` from `#[derive(Credentials)]`), the specialized
                    // inherent `marker_slice` on
                    // `CredentialsRegistryProbe<T: HasCredentialMarkers>`
                    // wins via inherent-first method resolution.
                    use #auth_path::CredentialsRegistryFallback as _;
                    // Autoref-specialization call site (per
                    // AUTHZ-CRED-MACRO-3): the inherent `marker_slice`
                    // emitted by `#[derive(Credentials)]` wins over the
                    // blanket trait default for types that derive, and
                    // falls back to `&[]` otherwise.
                    let __plexus_markers: &'static [#auth_path::CredentialFieldMarker] =
                        (&#auth_path::CredentialsRegistryProbe::<#probe_ty>::new())
                            .marker_slice();
                    let __plexus_method_path = #auth_path::MethodPath::try_new(#method_name)
                        .or_else(|_| #auth_path::MethodPath::try_new(
                            concat!("plexus.", #method_name)
                        ))
                        .unwrap_or_else(|_| #auth_path::MethodPath::try_new("plexus.unknown")
                            .expect("placeholder MethodPath always parses"));
                    let __plexus_issuer = #auth_path::CredentialIssuer::new(
                        #auth_path::Origin::new("plexus"),
                        __plexus_method_path,
                    );
                    __plexus_markers
                        .iter()
                        .map(|m| #crate_path::plexus::schema::CredentialFieldDecl::from_marker(
                            m, &[], __plexus_issuer.clone()
                        ))
                        .collect::<::std::vec::Vec<#crate_path::plexus::schema::CredentialFieldDecl>>()
                }
            };
            Some((quote! { #idx_lit }, expr))
        })
        .unzip();

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

    // IR-3: per-method deprecation token streams. For methods with no
    // `#[deprecated]` / `#[plexus_macros::removed_in]` annotation, we emit
    // nothing (the schema's `deprecation` field defaults to `None`).
    let (deprecation_index, deprecation_schema_calls): (Vec<_>, Vec<_>) = methods
        .iter()
        .enumerate()
        .filter_map(|(idx, m)| {
            let idx_lit = proc_macro2::Literal::usize_suffixed(idx);
            m.deprecation.as_ref().map(|dep| {
                (
                    quote! { #idx_lit },
                    deprecation_call(dep, crate_path),
                )
            })
        })
        .unzip();

    // IR-5: per-method `params_meta` token streams, carrying per-parameter
    // deprecation info. Only methods with at least one parameter that has a
    // non-`None` `ParsedDeprecation` emit a call; the rest leave the default
    // empty `Vec<ParamSchema>`.
    let (params_meta_index, params_meta_schema_calls): (Vec<_>, Vec<_>) = methods
        .iter()
        .enumerate()
        .filter_map(|(idx, m)| {
            let idx_lit = proc_macro2::Literal::usize_suffixed(idx);
            params_meta_call(m, crate_path).map(|call| (quote! { #idx_lit }, call))
        })
        .unzip();

    // IR-3: child-method schema entries. `#[child]` methods are NOT emitted as
    // RPC variants on the method enum (they don't take an RPC dispatch path),
    // but they DO contribute `MethodSchema` entries with `role = StaticChild`
    // / `DynamicChild { .. }` so `PluginSchema::is_hub_by_role()` returns
    // `true` for hubs whose children are declared via `#[child]`.
    let child_schema_entries: Vec<TokenStream> = child_methods
        .iter()
        .map(|c| child_schema_entry(c, crate_path))
        .collect();

    // REQ-6: per-method auth resolver entries — ((param_name, resolver_expr_string)).
    // Used at runtime to inject `x-plexus-source: { from: "auth", resolver: "..." }`
    // properties into the method's params schema.
    let auth_entries_per_method: Vec<TokenStream> = methods
        .iter()
        .map(|m| {
            let entries: Vec<TokenStream> = m.auth_resolvers
                .iter()
                .map(|r| {
                    let name = r.param_name.to_string();
                    let expr_str = r.resolver_expr.to_token_stream().to_string();
                    quote! { (#name, #expr_str) }
                })
                .collect();
            quote! { &[#(#entries),*] as &[(&str, &str)] }
        })
        .collect();

    // REQ-6: per-method override flag — `true` means `request = ()`, skip activation-level merge.
    let method_override_is_skip: Vec<bool> = methods
        .iter()
        .map(|m| matches!(m.request_override, Some(MethodRequestOverride::Skip)))
        .collect();

    // REQ-6: activation-level request schema token — `Some(schemars::schema_for!(T))` or `None`.
    let activation_request_schema_expr: TokenStream = match activation_request_type {
        Some(ty) => quote! { Some(#deps::schemars::schema_for!(#ty)) },
        None     => quote! { None },
    };

    // R-1 (AUTHZ-CORE-2): per-method explicit-scope emission. Methods that
    // declare `scope = "..."` (one or more) get a populated
    // `MethodSchema.requires_credential`:
    //
    // - one scope  → `RequiredCredential::from_method_scope(Scope::new(s))`
    // - N scopes   → `RequiredCredential::from_method_scopes(vec![..])`
    //                (conjunction — the caller must satisfy ALL)
    //
    // Methods with no `scope = "..."` emit nothing: the implicit
    // `<activation>.<method>` rule is resolved by AUTHZ-CORE-4's
    // `ScopeRegistry` at registry-build time, not carried in the IR
    // (superseding AUTHZ-CORE-7's `resolved_implicit_scope` — clients can
    // compute it from `activation.method` themselves).
    //
    // `Scope` is reached through plexus-core's re-export
    // (`#crate_path::plexus::schema::Scope`) so consumers don't need a
    // direct plexus-auth-core dependency, and umbrella-only consumers
    // resolve it through `::plexus_rpc::core` (Z2H-2).
    let (authz_scope_index, authz_scope_calls): (Vec<_>, Vec<_>) = methods
        .iter()
        .enumerate()
        .filter_map(|(idx, m)| {
            if m.scopes.is_empty() {
                return None;
            }
            let idx_lit = proc_macro2::Literal::usize_suffixed(idx);
            let call = if m.scopes.len() == 1 {
                let s = &m.scopes[0];
                quote! {
                    schema = schema.with_requires_credential(
                        #crate_path::plexus::schema::RequiredCredential::from_method_scope(
                            #crate_path::plexus::schema::Scope::new(#s),
                        ),
                    );
                }
            } else {
                let scope_news = m
                    .scopes
                    .iter()
                    .map(|s| quote! { #crate_path::plexus::schema::Scope::new(#s) });
                quote! {
                    schema = schema.with_requires_credential(
                        #crate_path::plexus::schema::RequiredCredential::from_method_scopes(
                            vec![#(#scope_news),*],
                        ),
                    );
                }
            };
            Some((quote! { #idx_lit }, call))
        })
        .unzip();

    // R-1: per-method `public` flags. Only `true` entries change the wire
    // (plexus-core skips serializing `public: false`), so pre-R-1 methods
    // keep their exact wire shape.
    let public_flags: Vec<bool> = methods.iter().map(|m| m.public).collect();

    // PLX-71: per-method open extension pairs, emitted via the parallel-slice
    // idiom (same shape as `public_flags` / `auth_entries_per_method` — a
    // per-method binding indexed by `i`, never the nested-zip tuple). Each
    // method's entry is a `vec![(key, value), ...]` whose values are
    // reconstructed as `serde_json::Value` tokens. Methods with no `ext(...)`
    // emit an empty vec, so their `MethodSchema.extensions` stays empty and
    // their wire bytes are unchanged.
    let extensions_per_method: Vec<TokenStream> = methods
        .iter()
        .map(|m| {
            let pairs: Vec<TokenStream> = m
                .extensions
                .iter()
                .map(|(k, v)| {
                    let value_tokens = ext_value_tokens(v, deps);
                    quote! { (#k, #value_tokens) }
                })
                .collect();
            quote! { vec![#(#pairs),*] }
        })
        .collect();

    // R-1 (joint AP-1): activation-declared posture stamp, applied to every
    // generated RPC method schema. Undeclared posture emits nothing.
    // Framework-injected entries (the auto `schema` method, `#[child]`
    // entries) are NOT stamped — the declaration describes the activation's
    // RPC enforcement surface.
    let posture_call: TokenStream = match auth_posture {
        Option::None => quote! {},
        Option::Some(p) => {
            let variant = match p {
                AuthPosture::Required => quote! { Required },
                AuthPosture::Optional => quote! { Optional },
                AuthPosture::Mixed => quote! { Mixed },
                AuthPosture::None => quote! { None },
            };
            quote! {
                schema = schema.with_auth_posture(
                    #crate_path::plexus::schema::AuthPosture::#variant,
                );
            }
        }
    };

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
                        .unwrap_or_else(|_| syn::parse_quote!(#deps::serde_json::Value));
                    let resp_ty: syn::Type = syn::parse_str(response)
                        .unwrap_or_else(|_| syn::parse_quote!(#deps::serde_json::Value));
                    Some((
                        quote! { #idx_lit },
                        quote! {
                            schema = schema
                                .with_bidirectional(true)
                                .with_request_type(#deps::schemars::schema_for!(#req_ty).into())
                                .with_response_type(#deps::schemars::schema_for!(#resp_ty).into());
                        },
                    ))
                }
            }
        })
        .unzip();

    quote! {
        /// Auto-generated method enum for schema extraction
        #[derive(Debug, Clone, #serde_derive_path::Serialize, #serde_derive_path::Deserialize, #schemars_derive_path::JsonSchema)]
        #[serde(tag = "method", content = "params", rename_all = "snake_case")]
        #derive_crate_attrs
        pub enum #enum_name {
            #(#variants),*
        }

        impl #enum_name {
            pub fn all_method_names() -> &'static [&'static str] {
                &[#(#method_names),*]
            }

            /// Cached schema value - computed once on first access
            fn cached_schema() -> &'static #deps::serde_json::Value {
                static SCHEMA_CACHE: std::sync::OnceLock<#deps::serde_json::Value> = std::sync::OnceLock::new();
                SCHEMA_CACHE.get_or_init(|| {
                    #deps::serde_json::to_value(#deps::schemars::schema_for!(#enum_name)).expect("Schema should serialize")
                })
            }
        }

        impl #crate_path::plexus::MethodEnumSchema for #enum_name {
            fn method_names() -> &'static [&'static str] {
                &[#(#method_names),*]
            }

            fn schema_with_consts() -> #deps::serde_json::Value {
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
                let return_schemas: Vec<(Option<#deps::schemars::Schema>, Vec<&str>)> = vec![#(#return_schema_entries),*];

                // REQ-6 runtime data: per-method auth resolvers, override flags, activation request schema.
                let auth_entries_per_method: Vec<&[(&str, &str)]> = vec![#(#auth_entries_per_method),*];
                let method_override_is_skip: &[bool] = &[#(#method_override_is_skip),*];
                // R-1: per-method explicit-public flags (AUTHZ-CORE-2).
                let public_flags: &[bool] = &[#(#public_flags),*];
                // PLX-71: per-method open extension pairs, indexed by `i`.
                let extensions_per_method: Vec<Vec<(&str, #deps::serde_json::Value)>> = vec![#(#extensions_per_method),*];
                let activation_request_schema: Option<#deps::schemars::Schema> = #activation_request_schema_expr;
                let activation_request_props: Option<#deps::serde_json::Map<String, #deps::serde_json::Value>> =
                    activation_request_schema
                        .as_ref()
                        .and_then(|s| #deps::serde_json::to_value(s).ok())
                        .and_then(|v| v.get("properties").cloned())
                        .and_then(|v| match v { #deps::serde_json::Value::Object(m) => Some(m), _ => None });

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
                                    #deps::serde_json::from_value::<#deps::schemars::Schema>(p).ok()
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

                        // REQ-6: augment the method's params schema with:
                        //   (a) #[from_auth(expr)] properties tagged x-plexus-source: { from: "auth", resolver: ... }
                        //   (b) activation-level request struct properties (when the activation declared `request = X`
                        //       and this method does NOT have `request = ()` override)
                        // Properties added here are NOT marked required — they are server-extracted.
                        let params = params.map(|p| {
                            let mut p_val: #deps::serde_json::Value = #deps::serde_json::to_value(&p).unwrap_or_default();
                            if let Some(obj) = p_val.as_object_mut() {
                                // Ensure properties object exists
                                let properties = obj
                                    .entry("properties".to_string())
                                    .or_insert_with(|| #deps::serde_json::Value::Object(#deps::serde_json::Map::new()));
                                if let #deps::serde_json::Value::Object(props_map) = properties {
                                    // (a) auth resolvers
                                    for (pname, rexpr) in auth_entries_per_method[i].iter() {
                                        props_map.insert(
                                            (*pname).to_string(),
                                            #deps::serde_json::json!({
                                                "x-plexus-source": {
                                                    "from": "auth",
                                                    "resolver": rexpr,
                                                }
                                            }),
                                        );
                                    }
                                    // (b) activation-level request fields (if present and not overridden)
                                    if !method_override_is_skip[i] {
                                        if let Some(req_props) = activation_request_props.as_ref() {
                                            for (fname, fschema) in req_props.iter() {
                                                // Don't clobber existing properties (e.g., an auth resolver
                                                // param happening to share a name).
                                                if !props_map.contains_key(fname) {
                                                    props_map.insert(fname.clone(), fschema.clone());
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            #deps::serde_json::from_value::<#deps::schemars::Schema>(p_val).unwrap_or(p)
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

                        // R-1 (joint AP-1): stamp the activation-declared
                        // auth posture (no-op when undeclared).
                        #posture_call

                        // R-1: explicit-public flag (AUTHZ-CORE-2). Only
                        // `true` reaches the wire (serde skips false).
                        if public_flags[i] {
                            schema = schema.with_public(true);
                        }

                        // PLX-71: apply per-method open extensions. Empty for
                        // methods with no `ext(...)`, so their wire shape is
                        // unchanged (plexus-core skips serializing an empty
                        // extensions map).
                        for (ext_key, ext_value) in extensions_per_method[i].iter() {
                            schema = schema.with_extension(*ext_key, ext_value.clone());
                        }

                        // R-1: explicit `scope = "..."` requirements →
                        // `requires_credential` (AUTHZ-CORE-2). Compile-time
                        // per-index match, same shape as the bidir calls.
                        match i {
                            #(
                                #authz_scope_index => {
                                    #authz_scope_calls
                                }
                            )*
                            _ => {}
                        }

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

                        // IR-3: apply deprecation metadata per-method. Emitted
                        // only for methods that carry `#[deprecated]` (see
                        // `parse_deprecation_attrs` for the contract).
                        match i {
                            #(
                                #deprecation_index => {
                                    #deprecation_schema_calls
                                }
                            )*
                            _ => {}
                        }

                        // IR-5: apply per-parameter deprecation metadata via
                        // `with_params_meta(..)`. Only methods with at least one
                        // parameter carrying `#[deprecated]` emit a call.
                        match i {
                            #(
                                #params_meta_index => {
                                    #params_meta_schema_calls
                                }
                            )*
                            _ => {}
                        }

                        // AUTHZ-CRED-MACRO-3: apply credential decls projected
                        // from the return type's `CredentialFieldMarker`
                        // registry (see `credential_decls_per_method` above).
                        //
                        // Like `bidir_schema_calls`, this is a per-index
                        // compile-time match so that type-level calls (here,
                        // `CredentialsRegistryProbe::<RT>::new()`) resolve at
                        // macro-expansion time against each method's concrete
                        // return type.
                        let __plexus_cred_decls: ::std::vec::Vec<#crate_path::plexus::schema::CredentialFieldDecl> = match i {
                            #(
                                #credential_decls_indices => #credential_decls_exprs,
                            )*
                            _ => ::std::vec::Vec::new(),
                        };
                        if !__plexus_cred_decls.is_empty() {
                            schema = schema.with_credentials(__plexus_cred_decls);
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

                // IR-3: append `#[child]` method entries. Each entry's role is
                // derived from the child method's signature: no extra arg →
                // `StaticChild`; `name: &str` arg → `DynamicChild { list_method,
                // search_method }` with values threaded through from
                // `#[child(list = "...", search = "...")]`.
                #(methods.push(#child_schema_entries);)*

                methods
            }

            /// Filter a return schema to only include specified variants
            ///
            /// This handles the case where a method returns an enum but only uses
            /// specific variants. The schema is filtered at runtime to only include
            /// those variants in the oneOf array.
            fn filter_return_schema(schema: #deps::schemars::Schema, allowed_variants: &[&str]) -> #deps::schemars::Schema {
                // Convert to JSON for manipulation
                let mut schema_value = #deps::serde_json::to_value(&schema).expect("Schema should serialize");

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
                #deps::serde_json::from_value(schema_value).expect("Filtered schema should deserialize")
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

/// IR-3: generate the `schema = schema.with_deprecation(DeprecationInfo { .. });`
/// token stream for a method whose `#[deprecated]` / `#[plexus_macros::removed_in]`
/// attributes have been folded into `ParsedDeprecation`.
fn deprecation_call(dep: &ParsedDeprecation, crate_path: &syn::Path) -> TokenStream {
    let since = &dep.since;
    let removed_in = &dep.removed_in;
    let message = &dep.message;
    quote! {
        schema = schema.with_deprecation(#crate_path::plexus::DeprecationInfo {
            since: ::std::string::String::from(#since),
            removed_in: ::std::string::String::from(#removed_in),
            message: ::std::string::String::from(#message),
        });
    }
}

/// IR-5: build the `schema = schema.with_params_meta(vec![ParamSchema { .. }, ..]);`
/// token stream for a method whose parameters carry per-param `#[deprecated]`
/// metadata. Returns `None` when no parameter on this method is deprecated —
/// the emitted schema leaves `params_meta` at its default empty `Vec`.
fn params_meta_call(m: &MethodInfo, crate_path: &syn::Path) -> Option<TokenStream> {
    let entries: Vec<TokenStream> = m
        .params
        .iter()
        .filter_map(|p| {
            let dep = p.deprecation.as_ref()?;
            let name = p.name.to_string();
            let since = &dep.since;
            let removed_in = &dep.removed_in;
            let message = &dep.message;
            Some(quote! {
                #crate_path::plexus::ParamSchema {
                    name: ::std::string::String::from(#name),
                    deprecation: ::std::option::Option::Some(
                        #crate_path::plexus::DeprecationInfo {
                            since: ::std::string::String::from(#since),
                            removed_in: ::std::string::String::from(#removed_in),
                            message: ::std::string::String::from(#message),
                        }
                    ),
                }
            })
        })
        .collect();

    if entries.is_empty() {
        None
    } else {
        Some(quote! {
            schema = schema.with_params_meta(::std::vec![
                #(#entries,)*
            ]);
        })
    }
}

/// IR-3: build a `MethodSchema::new(name, description, hash).with_role(..)
/// [.with_deprecation(..)]` expression for a `#[plexus_macros::child]` method.
///
/// The generated schema carries just enough metadata for downstream consumers
/// (synapse CLI, schema introspection) to recognize it as a child-routing
/// entry — the full RPC-method surface (params, returns, streaming, http_method)
/// doesn't apply to child methods.
fn child_schema_entry(child: &ChildMethodInfo, crate_path: &syn::Path) -> TokenStream {
    let name = child.fn_name.to_string();
    let description = &child.description;

    // Static children emit `MethodRole::StaticChild`; dynamic children emit
    // `MethodRole::DynamicChild { list_method, search_method }` with the
    // optional sibling-method names threaded through from the attribute.
    let role_expr = match child.kind {
        ChildMethodKind::Static => quote! { #crate_path::plexus::MethodRole::StaticChild },
        ChildMethodKind::Dynamic => {
            let list_expr = match &child.list_fn {
                Some(ident) => {
                    let s = ident.to_string();
                    quote! { ::std::option::Option::Some(::std::string::String::from(#s)) }
                }
                None => quote! { ::std::option::Option::None },
            };
            let search_expr = match &child.search_fn {
                Some(ident) => {
                    let s = ident.to_string();
                    quote! { ::std::option::Option::Some(::std::string::String::from(#s)) }
                }
                None => quote! { ::std::option::Option::None },
            };
            quote! {
                #crate_path::plexus::MethodRole::DynamicChild {
                    list_method: #list_expr,
                    search_method: #search_expr,
                }
            }
        }
    };

    // The `hash` for a child entry is stable w.r.t. the child's routing name;
    // there's no method signature to hash since these don't carry RPC params.
    // An empty hash matches the convention used by `plugin_children` synthesis
    // for `ChildSummary.hash` (see codegen/activation.rs CHILD-8 comments).
    let deprecation_application = if let Some(dep) = &child.deprecation {
        let call = deprecation_call(dep, crate_path);
        quote! { #call }
    } else {
        quote! {}
    };

    quote! {
        {
            let mut schema = #crate_path::plexus::MethodSchema::new(
                ::std::string::String::from(#name),
                ::std::string::String::from(#description),
                ::std::string::String::new(),
            );
            schema = schema.with_role(#role_expr);
            #deprecation_application
            schema
        }
    }
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

/// AUTHZ-CRED-MACRO-3: derive the "credential probe type" for a method —
/// the type that should be probed for `#[derive(Credentials)]` markers.
///
/// Resolution order:
/// 1. If the method has `stream_item_type` populated (streaming method
///    returning `impl Stream<Item = T>` or `impl Stream<Item = T> + Send`),
///    the probe type is `T`.
/// 2. Otherwise, if the return type is `Result<T, _>`, the probe type is `T`
///    (the Ok variant). This is the most common shape for non-streaming
///    methods.
/// 3. Otherwise, the probe type is the return type as-is.
/// 4. Returns `None` only when the return type contains a syntactic pattern
///    we don't know how to handle (e.g., bare `impl Trait` with no
///    extractable inner type, or a tuple). For those, the macro emits an
///    empty credential decls vec — same as pre-ticket behavior.
///
/// We deliberately do NOT recurse into `Option<T>`, `Vec<T>`, etc. The
/// autoref-probe handles the fallback uniformly: if the probe type doesn't
/// derive `Credentials`, the trait's empty-slice default applies and the
/// schema's `credentials` field stays empty.
fn effective_credential_probe_type(m: &MethodInfo) -> Option<Type> {
    // Streaming: probe the stream item type. The stream's `Item = T`
    // associated type IS the user's per-event payload, which is where
    // `#[derive(Credentials)]` lives in the streaming case.
    if let Some(item_ty) = &m.stream_item_type {
        return Some(item_ty.clone());
    }

    // Non-streaming: unwrap `Result<T, _>` to T if present, else use the
    // return type as-is. The recursion stops at one level — a user with
    // `Result<Result<T, E1>, E2>` would get the inner `Result<T, E1>`
    // probed, which falls through to empty via the trait default.
    if let Type::Path(tp) = &m.return_type {
        if let Some(seg) = tp.path.segments.last() {
            if seg.ident == "Result" {
                if let PathArguments::AngleBracketed(args) = &seg.arguments {
                    if let Some(GenericArgument::Type(ok_ty)) = args.args.first() {
                        return Some(ok_ty.clone());
                    }
                }
            }
        }
    }

    // Bare `impl Trait` returns aren't probable without the Stream-item
    // extraction having succeeded — there's no syntactic handle on the
    // concrete type. Emit `None` so the caller skips the credentials wiring.
    if matches!(&m.return_type, Type::ImplTrait(_)) {
        return None;
    }

    Some(m.return_type.clone())
}
