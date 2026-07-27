//! Generate Activation trait implementation and RPC server

use crate::parse::{
    ChildMethodInfo, ChildMethodKind, ListSearchMethodInfo, MethodInfo, ParsedDeprecation,
};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use uuid::Uuid;

pub fn generate(
    struct_name: &syn::Ident,
    self_ty: &syn::Type,
    impl_generics: &syn::ImplGenerics,
    where_clause: Option<&syn::WhereClause>,
    namespace: &str,
    version: Option<&str>,
    description: &str,
    long_description: Option<&str>,
    methods: &[MethodInfo],
    crate_path: &syn::Path,
    resolve_handle: bool,
    // CHILD-8: effective hub-mode — true when either `args.hub` is set OR
    // any `#[child]` method is present. Drives `call_fallback` (nested
    // routing) and `plugin_schema_body` (hub-shape schema).
    hub: bool,
    // CHILD-8: only the explicit `hub` flag from the activation attribute.
    // Used together with `child_methods.is_empty()` to detect the legacy
    // Solar-pre-migration pattern (`hub` flag + hand-written `ChildRouter`)
    // where the macro must NOT emit its own `ChildRouter` impl.
    hub_explicit: bool,
    // CHILD-8: when true, synthesize a `plugin_children(&self) -> Vec<ChildSummary>`
    // method from the static `#[child]` entries. Computed in `generate_all`
    // with knowledge of whether the user already defined one.
    synthesize_plugin_children: bool,
    plugin_id: Option<&str>,
    namespace_fn: Option<&str>,
    request_type: Option<&syn::Type>,
    children: &[syn::Ident],
    child_methods: &[ChildMethodInfo],
    list_method: Option<&ListSearchMethodInfo>,
    search_method: Option<&ListSearchMethodInfo>,
    // IR-5: activation-level `#[deprecated]` parsed off the impl block.
    activation_deprecation: Option<&ParsedDeprecation>,
    // Z2H-2: ident of the per-expansion dependency shim module emitted by
    // `generate_all`. All third-party names (jsonrpsee, serde_json, uuid,
    // async_trait, futures, tokio) are referenced relatively through it.
    deps: &syn::Ident,
    // PLX-91: the `ActivationIr` accessors, emitted into the inherent impl.
    activation_ir_fns: &TokenStream,
    // PLX-91: the handler table, emitted into the inherent impl.
    handler_table_fn: &TokenStream,
) -> TokenStream {
    let enum_name = format_ident!("{}Method", struct_name);
    let owned_wrapper_name = format_ident!("__PlexusOwnedChild{}", struct_name);
    let rpc_trait_name = format_ident!("{}Rpc", struct_name);
    let rpc_server_name = format_ident!("{}RpcServer", struct_name);
    let method_names: Vec<&str> = methods.iter().map(|m| m.method_name.as_str()).collect();

    // PLX-91 (the switchover): the only dispatch arms that survive are the
    // bidirectional ones. See `codegen::handlers` for why that construct alone
    // could not move onto the turn runtime, and why keeping it on its existing
    // path is the opposite of a silent semantic change.
    let dispatch_arms = generate_bidir_arms(methods, namespace, crate_path, request_type, deps);
    let help_arms = generate_help_arms(methods);
    let has_public_rpc_methods = methods.iter().any(|m| on_legacy_rpc_surface(m));
    let rpc_trait_methods = generate_rpc_trait_methods(methods, deps);
    let rpc_impl_methods = generate_rpc_impl_methods(struct_name, methods, namespace, crate_path, deps);

    // Conditionally generate resolve_handle method
    let resolve_handle_impl = if resolve_handle {
        quote! {
            async fn resolve_handle(
                &self,
                handle: &#crate_path::types::Handle,
            ) -> Result<#crate_path::plexus::PlexusStream, #crate_path::plexus::PlexusError> {
                self.resolve_handle_impl(handle).await
            }
        }
    } else {
        quote! {}
    };

    // Generate call() fallback - hub routes to children, leaf returns error
    let call_fallback = if hub {
        quote! {
            #crate_path::plexus::route_to_child(self, method, params, auth, raw_ctx).await
        }
    } else {
        quote! {
            Err(#crate_path::plexus::PlexusError::MethodNotFound {
                activation: #namespace.to_string(),
                method: method.to_string(),
            })
        }
    };

    // Generate long_description() method
    let long_description_impl = if let Some(long_desc) = long_description {
        quote! {
            fn long_description(&self) -> Option<&str> { Some(#long_desc) }
        }
    } else {
        quote! {
            fn long_description(&self) -> Option<&str> { None }
        }
    };

    // Generate version expression - either a literal string or env!("CARGO_PKG_VERSION")
    let version_expr = match version {
        Some(v) => quote! { #v },
        None => quote! { env!("CARGO_PKG_VERSION") },
    };

    // Resolve version string for UUID generation (needs a concrete value at proc-macro time)
    let version_for_uuid = version
        .map(|v| v.to_string())
        .unwrap_or_else(|| {
            // Read the calling crate's Cargo.toml to get the version
            std::env::var("CARGO_MANIFEST_DIR")
                .ok()
                .and_then(|dir| {
                    let cargo_toml = std::path::Path::new(&dir).join("Cargo.toml");
                    std::fs::read_to_string(cargo_toml).ok()
                })
                .as_deref()
                .and_then(package_version_from_cargo_toml)
                .unwrap_or_else(|| "0.0.0".to_string())
        });

    // Generate plugin_schema body - hub vs leaf, with or without long_description
    // When namespace_fn is set, use self.namespace() for runtime namespace
    let namespace_expr = if namespace_fn.is_some() {
        quote! { self.namespace() }
    } else {
        quote! { #namespace }
    };

    // Generate request schema injection — when `request = MyType` is set on the activation,
    // we embed the JSON Schema for that type in the plugin schema's `request` field.
    // We call <T as PlexusRequest>::request_schema() which is generated by #[derive(PlexusRequest)].
    let request_schema_inject = if let Some(req_ty) = request_type {
        quote! {
            {
                let req_value = <#req_ty as #crate_path::request::PlexusRequest>::request_schema();
                __plugin_schema.request = req_value;
            }
        }
    } else {
        quote! {}
    };

    // IR-5: if the impl block was annotated with `#[deprecated(...)]` the
    // codegen folded it into `activation_deprecation`. Emit the corresponding
    // `PluginSchema.deprecation` setter so the schema mirrors the Rust
    // deprecation at runtime.
    let deprecation_inject = if let Some(dep) = activation_deprecation {
        let since = &dep.since;
        let removed_in = &dep.removed_in;
        let message = &dep.message;
        quote! {
            __plugin_schema.deprecation = ::std::option::Option::Some(
                #crate_path::plexus::DeprecationInfo {
                    since: ::std::string::String::from(#since),
                    removed_in: ::std::string::String::from(#removed_in),
                    message: ::std::string::String::from(#message),
                }
            );
        }
    } else {
        quote! {}
    };

    let plugin_schema_body = match (hub, long_description) {
        (true, Some(long_desc)) => {
            // Hub with long description
            //
            // IR-16: `self.plugin_children()` is wrapped in an
            // `#[allow(deprecated)]` block so post-IR-4 activations do not
            // emit a "use of deprecated method" warning stemming from this
            // macro-emitted call. The suppression is narrowly scoped to the
            // macro's own emission — user code outside the macro is
            // unaffected.
            quote! {
                {
                    let __plugin_children = {
                        #[allow(deprecated)]
                        self.plugin_children()
                    };
                    let mut __plugin_schema = #crate_path::plexus::PluginSchema::hub_with_long_description(
                        #namespace_expr,
                        #version_expr,
                        #description,
                        #long_desc,
                        #enum_name::method_schemas(),
                        __plugin_children,
                    );
                    #request_schema_inject
                    #deprecation_inject
                    __plugin_schema
                }
            }
        }
        (true, None) => {
            // Hub without long description
            //
            // IR-16: `self.plugin_children()` wrapped in `#[allow(deprecated)]`
            // — see comment on the `(true, Some(_))` arm above.
            quote! {
                {
                    let __plugin_children = {
                        #[allow(deprecated)]
                        self.plugin_children()
                    };
                    let mut __plugin_schema = #crate_path::plexus::PluginSchema::hub(
                        #namespace_expr,
                        #version_expr,
                        #description,
                        #enum_name::method_schemas(),
                        __plugin_children,
                    );
                    #request_schema_inject
                    #deprecation_inject
                    __plugin_schema
                }
            }
        }
        (false, Some(long_desc)) => {
            // Leaf with long description
            quote! {
                {
                    let mut __plugin_schema = #crate_path::plexus::PluginSchema::leaf_with_long_description(
                        #namespace_expr,
                        #version_expr,
                        #description,
                        #long_desc,
                        #enum_name::method_schemas(),
                    );
                    #request_schema_inject
                    #deprecation_inject
                    __plugin_schema
                }
            }
        }
        (false, None) => {
            // Leaf without long description
            quote! {
                {
                    let mut __plugin_schema = #crate_path::plexus::PluginSchema::leaf(
                        #namespace_expr,
                        #version_expr,
                        #description,
                        #enum_name::method_schemas(),
                    );
                    #request_schema_inject
                    #deprecation_inject
                    __plugin_schema
                }
            }
        }
    };

    // Generate plugin_id - either from explicit value or deterministically from namespace+major_version
    // Using only major version ensures handles survive minor/patch upgrades (semver compatibility)
    let plugin_id_str = plugin_id
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            // Extract major version (first component before '.')
            let major_version = version_for_uuid.split('.').next().unwrap_or("0");
            // Generate deterministic UUID v5 from namespace@major_version
            let name = format!("{}@{}", namespace, major_version);
            Uuid::new_v5(&Uuid::NAMESPACE_OID, name.as_bytes()).to_string()
        });

    // Generate namespace() implementation - either constant or dynamic via method call
    let namespace_impl = if let Some(fn_name) = namespace_fn {
        let fn_ident = format_ident!("{}", fn_name);
        quote! { fn namespace(&self) -> &str { self.#fn_ident() } }
    } else {
        quote! { fn namespace(&self) -> &str { #namespace } }
    };

    // Generate jsonrpsee RPC trait + impl only if there are non-auth methods.
    // Auth-requiring methods are dispatched through Activation::call() which
    // handles auth injection. The jsonrpsee RPC trait cannot carry auth context.
    let into_rpc_body = if has_public_rpc_methods {
        quote! { self.into_rpc().into() }
    } else {
        quote! { #deps::jsonrpsee::core::server::Methods::new() }
    };

    let rpc_block = if has_public_rpc_methods {
        quote! {
            #[#deps::jsonrpsee::proc_macros::rpc(server, namespace = #namespace)]
            pub trait #rpc_trait_name {
                #(#rpc_trait_methods)*
            }

            #[#deps::async_trait::async_trait]
            impl #impl_generics #rpc_server_name for #self_ty #where_clause {
                #(#rpc_impl_methods)*
            }
        }
    } else {
        quote! {}
    };

    // Generate ChildRouter::get_child() body.
    //
    // Priority:
    //   1. If `#[child]` methods are present, dispatch via them (CHILD-3).
    //      Static methods match exact names; a dynamic method (if any)
    //      handles unmatched names by receiving the name.
    //   2. Else if `children = [field_a, field_b]` is set (legacy),
    //      dispatch by field name.
    //   3. Else return None (leaf activation).
    //
    // Note: codegen/mod.rs already errors if both `#[child]` methods and
    // `children = [...]` are present on the same impl.
    let get_child_body = if !child_methods.is_empty() {
        let static_children: Vec<&ChildMethodInfo> = child_methods
            .iter()
            .filter(|c| matches!(c.kind, ChildMethodKind::Static))
            .collect();
        let dynamic_child: Option<&ChildMethodInfo> = child_methods
            .iter()
            .find(|c| matches!(c.kind, ChildMethodKind::Dynamic));

        let static_names: Vec<String> =
            static_children.iter().map(|c| c.fn_name.to_string()).collect();
        let static_calls: Vec<TokenStream> = static_children
            .iter()
            .map(|c| {
                let fn_name = &c.fn_name;
                // Static child methods have shape `fn NAME(&self) -> Child`
                // which we always call synchronously. (Making static child
                // methods async would just force an unnecessary `.await`.)
                // PLX-91: the child is handed over OWNED, wrapped so that
                // `router_call` can reach `Activation::call_arc`. See
                // `owned_router_wrapper` for why the wrapper has to exist.
                quote! {
                    Some(::std::boxed::Box::new(#owned_wrapper_name(
                        ::std::sync::Arc::new(self.#fn_name())
                    )) as Box<dyn #crate_path::plexus::ChildRouter>)
                }
            })
            .collect();

        let default_arm = if let Some(dyn_child) = dynamic_child {
            let fn_name = &dyn_child.fn_name;
            let await_tok = if dyn_child.is_async {
                quote! { .await }
            } else {
                quote! {}
            };
            quote! {
                _ => self
                    .#fn_name(_name)
                    #await_tok
                    .map(|c| ::std::boxed::Box::new(#owned_wrapper_name(::std::sync::Arc::new(c)))
                        as Box<dyn #crate_path::plexus::ChildRouter>),
            }
        } else {
            quote! { _ => None, }
        };

        if static_children.is_empty() {
            // Pure-dynamic dispatch: no match needed.
            let fn_name = &dynamic_child.unwrap().fn_name;
            let await_tok = if dynamic_child.unwrap().is_async {
                quote! { .await }
            } else {
                quote! {}
            };
            quote! {
                self
                    .#fn_name(_name)
                    #await_tok
                    .map(|c| ::std::boxed::Box::new(#owned_wrapper_name(::std::sync::Arc::new(c)))
                        as Box<dyn #crate_path::plexus::ChildRouter>)
            }
        } else {
            quote! {
                match _name {
                    #(
                        #static_names => #static_calls,
                    )*
                    #default_arm
                }
            }
        }
    } else if children.is_empty() {
        quote! { None }
    } else {
        let child_names: Vec<String> = children.iter().map(|f| f.to_string()).collect();
        let child_fields: Vec<&syn::Ident> = children.iter().collect();
        quote! {
            match _name {
                #(
                    #child_names => Some(::std::boxed::Box::new(#owned_wrapper_name(
                        ::std::sync::Arc::new(self.#child_fields.clone())
                    )) as Box<dyn #crate_path::plexus::ChildRouter>),
                )*
                _ => None,
            }
        }
    };

    // CHILD-4: compute capabilities + list/search overrides.
    //
    // Capability rules:
    //   * Static `#[child]` methods are always listed — having any static
    //     child implies `ChildCapabilities::LIST`.
    //   * `#[child(list = "METHOD")]` adds `LIST` and delegates to the named
    //     sibling method (appended to static names when both are present).
    //   * `#[child(search = "METHOD")]` adds `SEARCH` and delegates to the
    //     named sibling method.
    //
    // Override emission rules:
    //   * `list_children()` is overridden iff LIST is set (static names or
    //     a list method, or both).
    //   * `search_children()` is overridden iff SEARCH is set.
    let static_children: Vec<&ChildMethodInfo> = child_methods
        .iter()
        .filter(|c| matches!(c.kind, ChildMethodKind::Static))
        .collect();
    let has_static_children = !static_children.is_empty();
    let list_enabled = has_static_children || list_method.is_some();
    let search_enabled = search_method.is_some();

    let capabilities_expr = match (list_enabled, search_enabled) {
        (false, false) => quote! { #crate_path::plexus::ChildCapabilities::empty() },
        (true, false) => quote! { #crate_path::plexus::ChildCapabilities::LIST },
        (false, true) => quote! { #crate_path::plexus::ChildCapabilities::SEARCH },
        (true, true) => quote! {
            #crate_path::plexus::ChildCapabilities::LIST | #crate_path::plexus::ChildCapabilities::SEARCH
        },
    };

    // Build a stream expression for the static child names (always listed),
    // and a stream expression for the user-supplied `list = "..."` method (if
    // any). When both are present they're chained; when only one is, we use
    // it directly. The final stream is always boxed as `BoxStream<'_, String>`.
    let static_names_strs: Vec<String> =
        static_children.iter().map(|c| c.fn_name.to_string()).collect();

    let list_children_impl = if list_enabled {
        let static_stream_expr = if has_static_children {
            // Emit `futures::stream::iter([...].into_iter().map(String::from))`.
            // We use a long-form path to avoid leaking extra imports into user
            // code.
            quote! {
                #deps::futures::stream::iter(
                    [#(#static_names_strs,)*].into_iter().map(::std::string::String::from)
                )
            }
        } else {
            quote! { #deps::futures::stream::empty::<::std::string::String>() }
        };

        let dynamic_list_expr = if let Some(lm) = list_method {
            let fn_name = &lm.fn_name;
            let await_tok = if lm.is_async { quote! { .await } } else { quote! {} };
            // `.boxed()` works for any `Stream<Item = String> + Send + 'a`.
            // Calling `.boxed()` on a `BoxStream<'_, String>` is a no-op
            // rewrap, which is fine.
            Some(quote! {
                {
                    use #deps::futures::stream::StreamExt;
                    self.#fn_name() #await_tok
                }
            })
        } else {
            None
        };

        let combined = match (has_static_children, dynamic_list_expr) {
            (true, Some(dyn_expr)) => {
                quote! {
                    {
                        use #deps::futures::stream::StreamExt;
                        let __static = #static_stream_expr;
                        let __dyn = #dyn_expr;
                        __static.chain(__dyn).boxed()
                    }
                }
            }
            (true, None) => {
                quote! {
                    {
                        use #deps::futures::stream::StreamExt;
                        #static_stream_expr.boxed()
                    }
                }
            }
            (false, Some(dyn_expr)) => {
                quote! {
                    {
                        use #deps::futures::stream::StreamExt;
                        #dyn_expr.boxed()
                    }
                }
            }
            (false, None) => unreachable!("list_enabled with no static or dynamic source"),
        };

        quote! {
            async fn list_children(&self) -> Option<#deps::futures::stream::BoxStream<'_, ::std::string::String>> {
                Some(#combined)
            }
        }
    } else {
        quote! {}
    };

    let search_children_impl = if let Some(sm) = search_method {
        let fn_name = &sm.fn_name;
        let await_tok = if sm.is_async { quote! { .await } } else { quote! {} };
        quote! {
            async fn search_children(&self, query: &str) -> Option<#deps::futures::stream::BoxStream<'_, ::std::string::String>> {
                use #deps::futures::stream::StreamExt;
                Some(self.#fn_name(query) #await_tok.boxed())
            }
        }
    } else {
        quote! {}
    };

    // Hub activations that use the legacy pattern (explicit `hub` flag with
    // hand-written `ChildRouter` impl, no `#[child]` methods — e.g. DynamicHub,
    // Solar-pre-migration) manage their own `ChildRouter` implementation with
    // custom routing logic. Skip generation only in that case.
    //
    // CHILD-8: when `#[child]` methods ARE present, we generate a `ChildRouter`
    // impl from them even if the activation is hub-mode — that's the whole
    // point of the attribute. Without this, a purely-`#[child]`-based hub
    // would have no router.
    let child_router_impl = if hub_explicit && child_methods.is_empty() {
        quote! {}
    } else {
        quote! {
            // Generated ChildRouter impl — enables this activation to participate in
            // hub routing without any hand-written boilerplate.
            #[#deps::async_trait::async_trait]
            impl #impl_generics #crate_path::plexus::ChildRouter for #self_ty #where_clause {
                fn router_namespace(&self) -> &str {
                    #namespace
                }

                async fn router_call(
                    &self,
                    method: &str,
                    params: #deps::serde_json::Value,
                    auth: Option<&#crate_path::plexus::AuthContext>,
                    raw_ctx: Option<&#crate_path::request::RawRequestContext>,
                ) -> Result<#crate_path::plexus::PlexusStream, #crate_path::plexus::PlexusError> {
                    <Self as #crate_path::plexus::Activation>::call(self, method, params, auth, raw_ctx).await
                }

                async fn get_child(&self, _name: &str) -> Option<Box<dyn #crate_path::plexus::ChildRouter>> {
                    #get_child_body
                }

                // CHILD-3/4: opt-in capability flags. Static children always
                // yield LIST; `list = "..."` / `search = "..."` opt in to the
                // corresponding flag. Otherwise empty.
                //
                // IR-16: `#[allow(deprecated)]` on the fn silences warnings
                // from the return type and the `ChildCapabilities::{empty,
                // LIST, SEARCH}` references in the body. The suppression is
                // scoped to this single generated fn; user code elsewhere is
                // unaffected. Dropped once `ChildCapabilities` is removed in
                // 0.7.
                #[allow(deprecated)]
                fn capabilities(&self) -> #crate_path::plexus::ChildCapabilities {
                    #capabilities_expr
                }

                #list_children_impl
                #search_children_impl
            }
        }
    };

    // CHILD-8: synthesize `plugin_children(&self) -> Vec<ChildSummary>` from
    // the static `#[child]` methods when the user hasn't defined one. Dynamic
    // `#[child]` methods are intentionally omitted — an open-ended name set
    // has no meaningful finite summary list; dynamic children are discoverable
    // via `ChildRouter::list_children` at runtime.
    //
    // The `hash` field of `ChildSummary` is populated with the empty string —
    // the macro has no compile-time hash source for the child plugin's schema.
    // Callers that need a populated hash hand-write `plugin_children` (which
    // suppresses synthesis) and compute the hash from the child's schema.
    let plugin_children_synth = if synthesize_plugin_children {
        let static_children: Vec<&ChildMethodInfo> = child_methods
            .iter()
            .filter(|c| matches!(c.kind, ChildMethodKind::Static))
            .collect();
        let entries: Vec<TokenStream> = static_children
            .iter()
            .map(|c| {
                let name = c.fn_name.to_string();
                let desc = &c.description;
                quote! {
                    #crate_path::plexus::ChildSummary {
                        namespace: ::std::string::String::from(#name),
                        description: ::std::string::String::from(#desc),
                        hash: ::std::string::String::new(),
                    }
                }
            })
            .collect();
        quote! {
            /// Auto-generated child summary list from `#[plexus_macros::child]`
            /// static methods. Dynamic children are not listed here; they are
            /// enumerable at runtime via `ChildRouter::list_children`.
            pub fn plugin_children(&self) -> ::std::vec::Vec<#crate_path::plexus::ChildSummary> {
                ::std::vec![
                    #(#entries,)*
                ]
            }
        }
    } else {
        quote! {}
    };

    // PLX-91: children are handed to `route_to_child` as `Box<dyn ChildRouter>`
    // and dispatched through `ChildRouter::router_call(&self, ..)` — a
    // *borrowed* seam, in plexus-core, which is read-only for this build. A
    // turn-native child needs an OWNED handle (PLX-97), so the box has to
    // contain something that already holds one. This wrapper is that thing: a
    // local generic newtype over `Arc<C>` whose `router_call` reaches
    // `Activation::call_arc`. It is local because coherence requires it —
    // `impl ChildRouter for Arc<C>` is an orphan impl (`Arc` is not
    // `#[fundamental]`), so a newtype is the only legal spelling. Every other
    // `ChildRouter` method forwards, so listing, searching, capabilities and
    // grandchild descent behave exactly as before.
    let owned_child_wrapper = quote! {
        #[doc(hidden)]
        #[allow(non_camel_case_types)]
        pub struct #owned_wrapper_name<C>(pub ::std::sync::Arc<C>);

        #[#deps::async_trait::async_trait]
        impl<C> #crate_path::plexus::ChildRouter for #owned_wrapper_name<C>
        where
            C: #crate_path::plexus::Activation
                + #crate_path::plexus::ChildRouter
                + ::core::marker::Sized,
        {
            fn router_namespace(&self) -> &str {
                #crate_path::plexus::ChildRouter::router_namespace(&*self.0)
            }

            async fn router_call(
                &self,
                method: &str,
                params: #deps::serde_json::Value,
                auth: Option<&#crate_path::plexus::AuthContext>,
                raw_ctx: Option<&#crate_path::request::RawRequestContext>,
            ) -> Result<#crate_path::plexus::PlexusStream, #crate_path::plexus::PlexusError> {
                <C as #crate_path::plexus::Activation>::call_arc(
                    ::std::sync::Arc::clone(&self.0), method, params, auth, raw_ctx,
                ).await
            }

            async fn get_child(&self, _name: &str) -> Option<Box<dyn #crate_path::plexus::ChildRouter>> {
                #crate_path::plexus::ChildRouter::get_child(&*self.0, _name).await
            }

            #[allow(deprecated)]
            fn capabilities(&self) -> #crate_path::plexus::ChildCapabilities {
                #crate_path::plexus::ChildRouter::capabilities(&*self.0)
            }

            async fn list_children(&self) -> Option<#deps::futures::stream::BoxStream<'_, ::std::string::String>> {
                #crate_path::plexus::ChildRouter::list_children(&*self.0).await
            }

            async fn search_children(&self, query: &str) -> Option<#deps::futures::stream::BoxStream<'_, ::std::string::String>> {
                #crate_path::plexus::ChildRouter::search_children(&*self.0, query).await
            }
        }
    };

    quote! {
        #owned_child_wrapper

        impl #impl_generics #self_ty #where_clause {
            pub const NAMESPACE: &'static str = #namespace;
            /// Stable plugin instance ID for handle routing
            pub const PLUGIN_ID: #deps::uuid::Uuid = #deps::uuid::uuid!(#plugin_id_str);

            /// Get the plugin's stable instance ID
            pub fn plugin_id(&self) -> #deps::uuid::Uuid {
                Self::PLUGIN_ID
            }

            /// PLX-91: hand this activation to a parent as an **owned** child
            /// router.
            ///
            /// `ChildRouter::router_call` takes `&self`, but a turn-native
            /// method needs an owned handle — the turn's stream outlives the
            /// borrow (PLX-97). `Box::new(self)` therefore no longer suffices
            /// for a macro-generated activation; this is the one-call
            /// replacement, and the macro's own generated `get_child` uses it
            /// for every `#[child]` edge. A hand-written `ChildRouter` should
            /// return `Some(Child.into_child_router())` where it used to
            /// return `Some(Box::new(Child))`.
            #[allow(dead_code)]
            pub fn into_child_router(self) -> ::std::boxed::Box<dyn #crate_path::plexus::ChildRouter>
            where
                Self: ::core::marker::Sized,
            {
                ::std::boxed::Box::new(#owned_wrapper_name(::std::sync::Arc::new(self)))
            }

            #plugin_children_synth

            #activation_ir_fns

            #handler_table_fn

            /// PLX-146: this activation's live turns, so a callback it issues
            /// can actually be answered.
            ///
            /// `call_arc` used to pass `live: None` to
            /// `turn_stream_to_plexus_stream`, which dropped the turn's
            /// `TurnControl` on the floor. The callback was still projected
            /// onto the wire as a `PlexusStreamItem::Request`, so from the
            /// outside it looked like the mechanism worked — but nothing held a
            /// control keyed by turn id, so no answer could ever be routed back
            /// and the handler's `client.*_async(..).await` never resolved.
            /// A generated activation could declare a callback and could never
            /// answer one.
            ///
            /// The registry is keyed by `TurnId`, which is a v4 UUID, so the
            /// `static` being per-monomorphization rather than per-instance
            /// cannot collide across activations. Entries are removed by the
            /// projection's terminal arm, so a resolved turn does not leak.
            #[doc(hidden)]
            #[allow(dead_code)]
            pub fn __plexus_live_turns() -> &'static #crate_path::runtime::LiveTurns {
                static __LIVE: ::std::sync::OnceLock<#crate_path::runtime::LiveTurns>
                    = ::std::sync::OnceLock::new();
                __LIVE.get_or_init(#crate_path::runtime::LiveTurns::new)
            }
        }

        #rpc_block

        #child_router_impl

        #[#deps::async_trait::async_trait]
        impl #impl_generics #crate_path::plexus::Activation for #self_ty #where_clause {
            type Methods = #enum_name;

            #namespace_impl
            fn version(&self) -> &str { #version_expr }
            fn description(&self) -> &str { #description }
            #long_description_impl

            fn methods(&self) -> Vec<&str> {
                vec![#(#method_names,)* "schema"]
            }

            fn method_help(&self, method: &str) -> Option<String> {
                match method {
                    #(#help_arms)*
                    "schema" => Some("Get plugin or method schema. Pass {\"method\": \"name\"} for a specific method.".to_string()),
                    _ => None,
                }
            }

            /// PLX-91: the borrowed entry point.
            ///
            /// `.schema`, child routing and the surviving bidirectional arms
            /// are served here, because none of them needs an owned handle on
            /// the activation. A turn-native `#[method]` does — the turn's
            /// stream outlives this borrow (PLX-97) — so it is served by
            /// [`Activation::call_arc`], which `DynamicHub` and the generated
            /// `ChildRouter` both route through.
            async fn call(
                &self,
                method: &str,
                params: #deps::serde_json::Value,
                auth: Option<&#crate_path::plexus::AuthContext>,
                raw_ctx: Option<&#crate_path::request::RawRequestContext>,
            ) -> Result<#crate_path::plexus::PlexusStream, #crate_path::plexus::PlexusError> {
                match method {
                    #(#dispatch_arms)*
                    "schema" => {
                        // PROT schema unification (PLX-13): `.schema` always yields the
                        // single unified PluginSchema. Per-method detail is read from
                        // `PluginSchema::methods`; there is no `SchemaResult::Method`
                        // variant and no `method`-param branch. An old client that still
                        // sends `{"method": ...}` gets the full plugin schema.
                        let result = #crate_path::plexus::SchemaResult::Plugin(self.plugin_schema());

                        Ok(#crate_path::plexus::wrap_stream(
                            #deps::futures::stream::once(async move { result }),
                            concat!(#namespace, ".schema"),
                            vec![#namespace.into()]
                        ))
                    }
                    _ => {
                        if Self::__plexus_handler_table().contains(method) {
                            return Err(#crate_path::plexus::PlexusError::ExecutionError(
                                format!(
                                    "PLX-91: `{}.{}` is a turn-native method and must be \
                                     dispatched through `Activation::call_arc(Arc<Self>, ..)`. \
                                     `call(&self, ..)` lends a borrow the turn's stream \
                                     outlives, so no owned handle exists to run the handler \
                                     with. `DynamicHub` and the generated `ChildRouter` \
                                     already route through `call_arc`.",
                                    #namespace, method,
                                )
                            ));
                        }
                        // PROT schema unification (PLX-13): the `{method}.schema`
                        // strip-suffix shortcut (and the HF-AUDIT-4 child-role guard
                        // that fenced it) is deleted. `<name>.schema` is no longer a
                        // per-method result path — it resolves to a child plugin's own
                        // `.schema` via child routing, or is method-not-found.
                        #call_fallback
                    }
                }
            }

            /// PLX-91 / PLX-97: **the** switched dispatch path.
            ///
            /// Three lines of actual work: look the method up in the
            /// `ActivationIr`, build the handler table, hand both plus the
            /// owned `Arc<Self>` to `runtime::entry_with_state`. Auth
            /// injection, capability pre-flight, the update/terminal split and
            /// the exactly-one-terminal guarantee all happen inside `entry` —
            /// there is no generated template for any of them.
            async fn call_arc(
                self: ::std::sync::Arc<Self>,
                method: &str,
                params: #deps::serde_json::Value,
                auth: Option<&#crate_path::plexus::AuthContext>,
                raw_ctx: Option<&#crate_path::request::RawRequestContext>,
            ) -> Result<#crate_path::plexus::PlexusStream, #crate_path::plexus::PlexusError> {
                let __handlers = Self::__plexus_handler_table();
                if __handlers.contains(method) {
                    let __ir = Self::__plexus_activation_ir_cached()
                        .map_err(#crate_path::plexus::PlexusError::ExecutionError)?;
                    let __dotted = __ir
                        .method(method)
                        .map(|m| m.dotted_id.clone())
                        .unwrap_or_else(|| format!("{}.{}", #namespace, method));
                    let __request = #crate_path::runtime::TurnRequest {
                        method: method.to_string(),
                        params,
                        auth: auth.cloned(),
                        raw_ctx: raw_ctx.cloned(),
                        peer: #crate_path::runtime::PeerCapabilities::all(),
                    };
                    let __handle = #crate_path::runtime::entry_with_state(
                        __ir, &__handlers, self, __request,
                    )?;
                    // PLX-146: register BEFORE the stream is handed back. The
                    // callback event can be observed by the peer as soon as the
                    // stream is polled, so a control registered afterwards
                    // would race a fast responder.
                    let __live = Self::__plexus_live_turns();
                    __live.register(&__handle);
                    return Ok(#crate_path::runtime::turn_stream_to_plexus_stream(
                        __dotted,
                        ::std::vec![#namespace.to_string()],
                        __handle,
                        ::core::option::Option::Some(::std::clone::Clone::clone(__live)),
                    ));
                }
                <Self as #crate_path::plexus::Activation>::call(
                    &*self, method, params, auth, raw_ctx,
                ).await
            }

            fn into_rpc_methods(self) -> #deps::jsonrpsee::core::server::Methods {
                #into_rpc_body
            }

            fn plugin_schema(&self) -> #crate_path::plexus::PluginSchema {
                #plugin_schema_body
            }

            #resolve_handle_impl
        }
    }
}

/// Whether a method appears on the legacy jsonrpsee surface.
///
/// This predicate had **three** copies — the `has_public_rpc_methods` guard,
/// the trait's method declarations, and the impl's method bodies — which is
/// exactly one copy per place PLX-146 had to change it, and the reason the
/// first attempt compiled the impl but not the trait (`E0046: not all trait
/// items implemented`). It is one function now, so the three cannot drift.
///
/// Excluded, all for the same reason — the legacy surface has no turn behind
/// it, so it cannot supply what the method needs, and `Activation::call`'s `_`
/// arm routes the caller to `call_arc` by name:
///
/// - `requires_auth`: no `AuthContext` here.
/// - `activation_params`: no request struct to extract from here.
/// - `capability_client` (PLX-146): no `TurnContext` to mint a `Client<C>`
///   from. An `unwired()` handle would compile and then fail at runtime on the
///   first callback with `CallbackError::NotWired`; routing is honest instead.
fn on_legacy_rpc_surface(m: &MethodInfo) -> bool {
    !m.requires_auth && m.activation_params.is_empty() && m.capability_client.is_none()
}

fn generate_rpc_trait_methods(methods: &[MethodInfo], deps: &syn::Ident) -> Vec<TokenStream> {
    methods
        .iter()
        // Auth-requiring methods and methods with activation_params are dispatched through
        // Activation::call() which handles auth injection and request extraction.
        // The jsonrpsee RPC trait cannot carry auth context or raw_ctx.
        .filter(|m| on_legacy_rpc_surface(m))
        .map(|m| {
            let method_name = &m.fn_name;
            let method_name_str = &m.method_name;
            let unsubscribe_name = format!("unsubscribe_{}", method_name_str);
            let doc = &m.description;

            let params: Vec<TokenStream> = m
                .params
                .iter()
                .map(|p| {
                    let name = &p.name;
                    let ty = &p.ty;
                    quote! { #name: #ty }
                })
                .collect();

            quote! {
                #[doc = #doc]
                #[subscription(name = #method_name_str, unsubscribe = #unsubscribe_name, item = #deps::serde_json::Value)]
                async fn #method_name(&self, #(#params),*) -> #deps::jsonrpsee::core::SubscriptionResult;
            }
        })
        .collect()
}

fn generate_rpc_impl_methods(
    struct_name: &syn::Ident,
    methods: &[MethodInfo],
    namespace: &str,
    crate_path: &syn::Path,
    deps: &syn::Ident,
) -> Vec<TokenStream> {
    use crate::parse::BidirType;

    methods
        .iter()
        .filter(|m| on_legacy_rpc_surface(m))
        .map(|m| {
            let method_name = &m.fn_name;
            let method_name_str = &m.method_name;
            let param_names: Vec<&syn::Ident> = m.params.iter().map(|p| &p.name).collect();

            let params_with_types: Vec<TokenStream> = m
                .params
                .iter()
                .map(|p| {
                    let name = &p.name;
                    let ty = &p.ty;
                    quote! { #name: #ty }
                })
                .collect();

            // Generate bidirectional channel setup and method call based on bidir type
            let (bidir_setup, method_call) = match &m.bidirectional {
                BidirType::None => {
                    (quote! {}, quote! { #struct_name::#method_name(self, #(#param_names),*).await })
                }
                BidirType::Standard => {
                    (
                        quote! {
                            let (tx, _rx) = #deps::tokio::sync::mpsc::channel(32);
                            let ctx = std::sync::Arc::new(#crate_path::plexus::BidirChannel::<
                                #crate_path::plexus::StandardRequest,
                                #crate_path::plexus::StandardResponse,
                            >::new(
                                tx,
                                false,
                                vec![#namespace.into()],
                                #crate_path::plexus::PlexusContext::hash(),
                            ));
                        },
                        quote! { #struct_name::#method_name(self, &ctx, #(#param_names),*).await }
                    )
                }
                BidirType::Custom { request, response } => {
                    let req_ty: syn::Type = syn::parse_str(request).unwrap_or_else(|_| syn::parse_quote!(()));
                    let resp_ty: syn::Type = syn::parse_str(response).unwrap_or_else(|_| syn::parse_quote!(()));
                    (
                        quote! {
                            let (tx, _rx) = #deps::tokio::sync::mpsc::channel(32);
                            let ctx = std::sync::Arc::new(#crate_path::plexus::BidirChannel::<
                                #req_ty,
                                #resp_ty,
                            >::new(
                                tx,
                                false,
                                vec![#namespace.into()],
                                #crate_path::plexus::PlexusContext::hash(),
                            ));
                        },
                        quote! { #struct_name::#method_name(self, &ctx, #(#param_names),*).await }
                    )
                }
            };

            // All methods use wrap_stream for consistent streaming pattern
            let content_type = format!("{}.{}", namespace, method_name_str);

            // PLX-110: the SECOND site that has to branch on the return shape.
            //
            // The ticket names `codegen/handlers.rs`, and that is where the
            // turn-native dispatch lives — but the probe's four compile errors
            // pointed at `plexus-core/src/plexus/streaming.rs:55` as well as at
            // the handler table, because this legacy jsonrpsee subscription
            // impl feeds the method's return value straight into
            // `wrap_stream<T: Serialize>(impl Stream<Item = T>, ..)`. A unary
            // method that only fixed `handlers.rs` would still not compile.
            //
            // What a unary method looks like on THIS surface: a subscription
            // has no terminal channel — it has Data items and a Done — so the
            // `Ok` value arrives as one Data item and then Done, and the `Err`
            // arrives as one Data item carrying the serialized `TurnError`
            // envelope. That is lossier than the turn projection (a consumer
            // must read the envelope to see it failed), and it is a residual
            // recorded rather than hidden: this surface predates the turn model
            // and PLX-91 kept it deliberately. Both arms unify at
            // `PlexusStream`, and `wrap_stream` still does the serialization,
            // so `Credential<T>` capture (AUTHZ-CRED-CORE-2) keeps working on
            // the Ok path.
            let wrapped_stream = {
                use crate::ir_parse::signature::DispatchShape;
                // PLX-112: this site goes through `IntoTurnStop` too, for the
                // same reason PLX-110 had to touch it at all — a refusal type
                // that only implements `IntoTurnStop` does not satisfy
                // `Into<TurnError>`, so fixing only `handlers.rs` would have
                // made the whole activation fail to compile. The macro still
                // guesses nothing here: it delegates to the same trait.
                //
                // This surface has no terminal and therefore no `StopReason`,
                // so it cannot express the kind at all — a `Refused` arrives as
                // one Data item carrying the `StopDetail`, exactly as a
                // `Failed` arrives as one Data item carrying the `TurnError`.
                // That is the *same* pre-existing lossiness PLX-110 recorded
                // for the error arm, not a new one, and it is why the turn
                // projection is the surface the RFC constrains.
                let once_err = quote! {
                    ::core::result::Result::Err(__e) => {
                        match #crate_path::runtime::IntoTurnStop::into_turn_stop(__e) {
                            #crate_path::runtime::TurnStop::Failed(__te) => {
                                #crate_path::plexus::wrap_stream(
                                    #deps::futures::stream::once(async move { __te }),
                                    #content_type,
                                    vec![#namespace.into()]
                                )
                            }
                            #crate_path::runtime::TurnStop::Refused(__d)
                            | #crate_path::runtime::TurnStop::Limited(__d) => {
                                #crate_path::plexus::wrap_stream(
                                    #deps::futures::stream::once(async move { __d }),
                                    #content_type,
                                    vec![#namespace.into()]
                                )
                            }
                        }
                    }
                };
                match m.dispatch {
                    // Byte-for-byte the pre-PLX-110 emission.
                    DispatchShape::Stream | DispatchShape::Other => quote! {
                        let stream = #method_call;
                        let wrapped = #crate_path::plexus::wrap_stream(
                            stream,
                            #content_type,
                            vec![#namespace.into()]
                        );
                    },
                    DispatchShape::ResultStream => quote! {
                        let wrapped = match #method_call {
                            ::core::result::Result::Ok(__s) => #crate_path::plexus::wrap_stream(
                                __s,
                                #content_type,
                                vec![#namespace.into()]
                            ),
                            #once_err
                        };
                    },
                    DispatchShape::UnaryValue => quote! {
                        let wrapped = match #method_call {
                            ::core::result::Result::Ok(__v) => #crate_path::plexus::wrap_stream(
                                #deps::futures::stream::once(async move { __v }),
                                #content_type,
                                vec![#namespace.into()]
                            ),
                            #once_err
                        };
                    },
                    // `Result<(), E>`: no value to send, so the subscription is
                    // just Done — the closest this surface gets to "a terminal
                    // with no value" (RFC 002 §6.4).
                    DispatchShape::UnaryUnit => quote! {
                        let wrapped = match #method_call {
                            ::core::result::Result::Ok(_) => #crate_path::plexus::wrap_stream(
                                #deps::futures::stream::empty::<#deps::serde_json::Value>(),
                                #content_type,
                                vec![#namespace.into()]
                            ),
                            #once_err
                        };
                    },
                }
            };

            quote! {
                async fn #method_name(
                    &self,
                    pending: #deps::jsonrpsee::PendingSubscriptionSink,
                    #(#params_with_types),*
                ) -> #deps::jsonrpsee::core::SubscriptionResult {
                    use #deps::futures::StreamExt;

                    let sink = pending.accept().await?;
                    #bidir_setup
                    #wrapped_stream

                    #deps::tokio::spawn(async move {
                        let mut stream = wrapped;
                        while let Some(item) = stream.next().await {
                            if let Ok(raw_value) = #deps::serde_json::value::to_raw_value(&item) {
                                if sink.send(raw_value).await.is_err() {
                                    break;
                                }
                            }
                        }
                        // Send done event
                        let done = #crate_path::plexus::PlexusStreamItem::Done {
                            metadata: #crate_path::plexus::StreamMetadata::new(
                                vec![#namespace.into()],
                                #crate_path::plexus::PlexusContext::hash(),
                            ),
                        };
                        if let Ok(raw_value) = #deps::serde_json::value::to_raw_value(&done) {
                            let _ = sink.send(raw_value).await;
                        }
                    });

                    Ok(())
                }
            }
        })
        .collect()
}

/// PLX-91: the residue of `generate_dispatch_arms`.
///
/// Before the switchover this function produced one arm per `#[method]`, each
/// re-emitting auth injection, request extraction, param decode and
/// wrap/terminal plumbing. All four now live once, in
/// `plexus_core::runtime::entry`, and the arm list is empty for every method
/// the handler table serves. What is left is the `BidirType::{Standard,
/// Custom}` pair, which `codegen::handlers`' module docs explain cannot move
/// without inventing a capability type per request/response pair — and which
/// ACP depends on, so it is kept working rather than deleted.
fn generate_bidir_arms(
    methods: &[MethodInfo],
    namespace: &str,
    crate_path: &syn::Path,
    request_type: Option<&syn::Type>,
    deps: &syn::Ident,
) -> Vec<TokenStream> {
    use crate::parse::BidirType;

    methods
        .iter()
        .filter(|m| !crate::codegen::handlers::is_switched(m))
        .map(|m| {
            let method_name = &m.method_name;
            let fn_name = &m.fn_name;

            // Generate param extraction code
            let (param_extraction, param_names) = generate_param_extraction(m, crate_path, deps);

            let content_type = format!("{}.{}", namespace, method_name);

            // Generate auth injection code if needed.
            //
            // AUTHZ-MACRO-OPTIONAL-AUTH-1: branch on the parsed shape.
            //   - `auth: &AuthContext`            → unwrap or `Unauthenticated`
            //   - `auth: Option<&AuthContext>`    → pass `Option` straight through
            // `auth_ctx` is the name handed to the user's method. In the
            // optional case, it is itself an `Option<&AuthContext>`, which is
            // exactly the type the handler asked for. `#[from_auth]` resolver
            // bindings always run with `&auth_ctx`, so they need the
            // dereferenced form — that path stays on the force-unwrap branch.
            let auth_injection = if m.requires_auth {
                if m.auth_optional && m.auth_resolvers.is_empty() {
                    // Optional-auth: hand the user the raw `Option<&AuthContext>`.
                    quote! {
                        let auth_ctx: ::std::option::Option<&#crate_path::plexus::AuthContext> = auth;
                    }
                } else {
                    // Force-auth (or any path that uses #[from_auth] resolvers,
                    // which expect a concrete `&AuthContext`): unwrap or error.
                    quote! {
                        let auth_ctx = auth.ok_or_else(|| #crate_path::plexus::PlexusError::Unauthenticated("This method requires authentication".to_string()))?;
                    }
                }
            } else {
                quote! {}
            };

            // Generate #[from_auth(resolver)] calls
            // Each resolver is called with &auth_ctx and produces a named binding via ?
            let resolver_bindings: Vec<proc_macro2::TokenStream> = m.auth_resolvers.iter().map(|r| {
                let name = &r.param_name;
                let expr = &r.resolver_expr;
                quote! {
                    let #name = #expr(&auth_ctx).await
                        .map_err(|e| #crate_path::plexus::PlexusError::ExecutionError(e.to_string()))?;
                }
            }).collect();

            // Determine whether to skip activation-level request extraction.
            // `request = ()` on a method means: bypass extraction even if the activation
            // declares `request = MyType`.
            let skip_extraction = matches!(
                &m.request_override,
                Some(crate::parse::MethodRequestOverride::Skip)
            );

            // Generate #[activation_param] extraction from request struct.
            // When the activation declares `request = MyType`, we extract the request struct
            // from raw_ctx and pull named fields from it.
            let activation_param_bindings: Vec<proc_macro2::TokenStream> = if !m.activation_params.is_empty() && !skip_extraction {
                m.activation_params.iter().map(|ap| {
                    let name = &ap.param_name;
                    let ty = &ap.ty;
                    quote! {
                        let #name: #ty = req.#name.clone();
                    }
                }).collect()
            } else {
                Vec::new()
            };

            // If there are activation_params, generate request extraction code.
            // This uses the activation-level request type passed in.
            // When request_type is set but no activation_params are used, we still
            // extract for validation (e.g. origin gating) and discard the value.
            let activation_extraction = if !skip_extraction {
                if let Some(req_ty) = request_type {
                    if !m.activation_params.is_empty() {
                        // Extract and bind to `req` so activation_param_bindings can use it
                        quote! {
                            let __raw_ctx = raw_ctx
                                .ok_or_else(|| #crate_path::plexus::PlexusError::Unauthenticated("no request context".into()))?;
                            let req = <#req_ty as #crate_path::request::PlexusRequest>::extract(__raw_ctx)
                                .map_err(|e| #crate_path::plexus::PlexusError::ExecutionError(e.to_string()))?;
                        }
                    } else {
                        // No activation_params but request_type is set — extract for
                        // validation only (e.g. origin gating), discard the value.
                        quote! {
                            if let Some(__raw_ctx) = raw_ctx {
                                <#req_ty as #crate_path::request::PlexusRequest>::extract(__raw_ctx)?;
                            }
                        }
                    }
                } else {
                    // No request type declared but activation_params used — this will produce
                    // a compile error naturally since `req` won't be defined.
                    quote! {}
                }
            } else {
                quote! {}
            };

            // Build parameter list for method call
            let activation_param_names: Vec<&syn::Ident> = m.activation_params.iter().map(|ap| &ap.param_name).collect();
            let resolver_names: Vec<&syn::Ident> = m.auth_resolvers.iter().map(|r| &r.param_name).collect();
            let _method_params = if m.requires_auth && m.auth_resolvers.is_empty() {
                // Raw auth context pass-through (legacy)
                quote! { auth_ctx, #(#param_names),* }
            } else if !m.activation_params.is_empty() && !m.auth_resolvers.is_empty() {
                // Both activation_params and resolver params + regular params
                quote! { #(#activation_param_names,)* #(#resolver_names,)* #(#param_names),* }
            } else if !m.activation_params.is_empty() {
                // activation_params + regular params
                quote! { #(#activation_param_names,)* #(#param_names),* }
            } else if !m.auth_resolvers.is_empty() {
                // Resolved parameters + regular params
                quote! { #(#resolver_names,)* #(#param_names),* }
            } else {
                quote! { #(#param_names),* }
            };

            // Generate code based on bidirectional type
            match &m.bidirectional {
                // Unreachable: filtered out above. The switchover serves every
                // non-bidirectional method from the handler table.
                BidirType::None => quote! {},
                BidirType::Standard => {
                    // Build param list for bidir - ctx comes first, then auth if needed, then regular params
                    let bidir_params = if m.requires_auth {
                        quote! { &ctx, auth_ctx, #(#param_names),* }
                    } else {
                        quote! { &ctx, #(#param_names),* }
                    };

                    // StandardBidirChannel - use wrap_stream_with_bidir
                    quote! {
                        #method_name => {
                            #auth_injection
                            #activation_extraction
                            #(#activation_param_bindings)*
                            #(#resolver_bindings)*
                            #param_extraction
                            // Create bidir channel and wrap stream together
                            let (ctx, wrap_fn) = #crate_path::plexus::create_bidir_stream::<
                                #crate_path::plexus::StandardRequest,
                                #crate_path::plexus::StandardResponse,
                            >(#content_type, vec![#namespace.into()]);

                            // Call the method with the bidir channel
                            let user_stream = self.#fn_name(#bidir_params).await;

                            // Wrap the user's stream and merge with bidir requests
                            let wrapped_user = #crate_path::plexus::wrap_stream(
                                user_stream,
                                #content_type,
                                vec![#namespace.into()]
                            );
                            Ok(wrap_fn(wrapped_user))
                        }
                    }
                }
                BidirType::Custom { request, response } => {
                    // Build param list for bidir - ctx comes first, then auth if needed, then regular params
                    let bidir_params = if m.requires_auth {
                        quote! { &ctx, auth_ctx, #(#param_names),* }
                    } else {
                        quote! { &ctx, #(#param_names),* }
                    };

                    // Custom BidirChannel - use wrap_stream_with_bidir with custom types
                    let req_ty: syn::Type = syn::parse_str(request).unwrap_or_else(|_| syn::parse_quote!(()));
                    let resp_ty: syn::Type = syn::parse_str(response).unwrap_or_else(|_| syn::parse_quote!(()));
                    quote! {
                        #method_name => {
                            #auth_injection
                            #activation_extraction
                            #(#activation_param_bindings)*
                            #(#resolver_bindings)*
                            #param_extraction
                            // Create bidir channel and wrap stream together
                            let (ctx, wrap_fn) = #crate_path::plexus::create_bidir_stream::<
                                #req_ty,
                                #resp_ty,
                            >(#content_type, vec![#namespace.into()]);

                            // Call the method with the bidir channel
                            let user_stream = self.#fn_name(#bidir_params).await;

                            // Wrap the user's stream and merge with bidir requests
                            let wrapped_user = #crate_path::plexus::wrap_stream(
                                user_stream,
                                #content_type,
                                vec![#namespace.into()]
                            );
                            Ok(wrap_fn(wrapped_user))
                        }
                    }
                }
            }
        })
        .collect()
}

/// Generate param extraction code and return the param names for the call
fn generate_param_extraction<'a>(m: &'a MethodInfo, crate_path: &syn::Path, deps: &syn::Ident) -> (TokenStream, Vec<&'a syn::Ident>) {
    let param_names: Vec<&syn::Ident> = m.params.iter().map(|p| &p.name).collect();

    let extraction = match m.params.len() {
        0 => quote! {},
        1 => {
            let param = &m.params[0];
            let param_name = &param.name;
            let param_str = param_name.to_string();
            let is_option = crate::codegen::method_enum::is_option_type(&param.ty);

            if is_option {
                // For Option<T>, missing field OR explicit null = None (not an error)
                quote! {
                    let #param_name = match &params {
                        #deps::serde_json::Value::Object(map) => {
                            map.get(#param_str)
                                .filter(|v| !v.is_null())  // Treat explicit null as missing
                                .map(|v| #deps::serde_json::from_value(v.clone()))
                                .transpose()
                                .map_err(|e| #crate_path::plexus::PlexusError::InvalidParams(e.to_string()))?
                        }
                        #deps::serde_json::Value::Null => None,
                        _ => #deps::serde_json::from_value(params.clone())
                            .map_err(|e| #crate_path::plexus::PlexusError::InvalidParams(e.to_string()))?,
                    };
                }
            } else {
                // For required param, try to extract from object or deserialize directly
                quote! {
                    let #param_name = match &params {
                        #deps::serde_json::Value::Object(map) => {
                            if let Some(val) = map.get(#param_str) {
                                #deps::serde_json::from_value(val.clone())
                                    .map_err(|e| #crate_path::plexus::PlexusError::InvalidParams(e.to_string()))?
                            } else {
                                return Err(#crate_path::plexus::PlexusError::InvalidParams(
                                    format!("missing field: {}", #param_str)
                                ));
                            }
                        }
                        _ => #deps::serde_json::from_value(params.clone())
                            .map_err(|e| #crate_path::plexus::PlexusError::InvalidParams(e.to_string()))?,
                    };
                }
            }
        }
        _ => {
            let extractions: Vec<TokenStream> = m
                .params
                .iter()
                .map(|p| {
                    let name = &p.name;
                    let name_str = name.to_string();
                    let is_option = crate::codegen::method_enum::is_option_type(&p.ty);

                    if is_option {
                        // For Option<T>, missing field OR explicit null = None (not an error)
                        quote! {
                            let #name = map.get(#name_str)
                                .filter(|v| !v.is_null())  // Treat explicit null as missing
                                .map(|v| #deps::serde_json::from_value(v.clone()))
                                .transpose()
                                .map_err(|e| #crate_path::plexus::PlexusError::InvalidParams(e.to_string()))?;
                        }
                    } else {
                        // For required fields, error on missing
                        quote! {
                            let #name = map.get(#name_str)
                                .ok_or_else(|| #crate_path::plexus::PlexusError::InvalidParams(
                                    format!("missing field: {}", #name_str)
                                ))
                                .and_then(|v| #deps::serde_json::from_value(v.clone())
                                    .map_err(|e| #crate_path::plexus::PlexusError::InvalidParams(e.to_string())))?;
                        }
                    }
                })
                .collect();

            quote! {
                let map = params.as_object()
                    .ok_or_else(|| #crate_path::plexus::PlexusError::InvalidParams(
                        "expected object".to_string()
                    ))?;
                #(#extractions)*
            }
        }
    };

    (extraction, param_names)
}

fn generate_help_arms(methods: &[MethodInfo]) -> Vec<TokenStream> {
    methods
        .iter()
        .map(|m| {
            let name = &m.method_name;
            let desc = &m.description;
            quote! { #name => Some(#desc.to_string()), }
        })
        .collect()
}

/// PLX-92: resolve the *package* version from a `Cargo.toml`'s text.
///
/// The previous implementation took the first line whose trimmed form started
/// with `version` and read the first quoted token off it. That is wrong for any
/// manifest whose `[dependencies]` (or `[build-dependencies]`, or a
/// `[dependencies.foo]` sub-table) precedes `[package]`, or whose `[package]`
/// section is not first in the file — it happily returned a *dependency's*
/// version and baked it into the activation UUID.
///
/// This scan is section-aware: it tracks the current TOML table header and only
/// accepts a `version = "..."` key while inside `[package]`. When `[package]`
/// inherits from the workspace (`version.workspace = true`, which has no string
/// literal), it falls back to `[workspace.package]`.
///
/// Still deliberately a line scanner, not a TOML parser — proc-macro crates pay
/// for every dependency, and a manifest's `[package] version` is a one-line key
/// in every manifest rustc itself will accept.
fn package_version_from_cargo_toml(content: &str) -> Option<String> {
    let mut package_version: Option<String> = None;
    let mut workspace_package_version: Option<String> = None;
    let mut section = String::new();

    for raw in content.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            // Table header. `[[bin]]` array-of-table headers land here too and
            // simply set a section name that never matches.
            if let Some(end) = line.find(']') {
                section = line[1..end].trim().trim_matches('"').to_string();
            }
            continue;
        }
        if section != "package" && section != "workspace.package" {
            continue;
        }
        // Split off the key. `version.workspace = true` has key
        // `version.workspace`, which must NOT match.
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() != "version" {
            continue;
        }
        // Take the first quoted token from the value side only, so a trailing
        // `# comment "1.0.0"` cannot win.
        let value = value.split('#').next().unwrap_or(value);
        let Some(v) = value.split('"').nth(1) else {
            continue;
        };
        if section == "package" {
            if package_version.is_none() {
                package_version = Some(v.to_string());
            }
        } else if workspace_package_version.is_none() {
            workspace_package_version = Some(v.to_string());
        }
    }

    package_version.or(workspace_package_version)
}

#[cfg(test)]
mod version_scan_tests {
    use super::package_version_from_cargo_toml;

    /// The PLX-92 acceptance case: `[dependencies]` precedes `[package]`, so the
    /// first `version =` line in the file belongs to a dependency. The naive
    /// scan returned "9.9.9"; the section-aware scan must return "1.2.3".
    #[test]
    fn dependencies_section_before_package_does_not_win() {
        let manifest = r#"
[dependencies]
serde = { version = "9.9.9", features = ["derive"] }

[dependencies.tokio]
version = "8.8.8"

[package]
name = "my-activation"
version = "1.2.3"
edition = "2021"
"#;
        assert_eq!(
            package_version_from_cargo_toml(manifest).as_deref(),
            Some("1.2.3"),
            "the [package] version must win over any dependency's version"
        );
    }

    #[test]
    fn package_first_still_resolves() {
        let manifest = r#"
[package]
name = "my-activation"
version = "0.6.0"

[dependencies]
serde = { version = "1.0.200" }
"#;
        assert_eq!(
            package_version_from_cargo_toml(manifest).as_deref(),
            Some("0.6.0")
        );
    }

    #[test]
    fn workspace_inherited_version_falls_back_to_workspace_package() {
        let manifest = r#"
[dependencies]
serde = { version = "9.9.9" }

[package]
name = "my-activation"
version.workspace = true

[workspace.package]
version = "2.1.0"
"#;
        assert_eq!(
            package_version_from_cargo_toml(manifest).as_deref(),
            Some("2.1.0"),
            "`version.workspace = true` must not match, and the workspace value must be used"
        );
    }

    #[test]
    fn commented_out_version_is_ignored() {
        let manifest = r#"
[package]
name = "my-activation"
# version = "6.6.6"
version = "3.0.1"  # not "7.7.7"
"#;
        assert_eq!(
            package_version_from_cargo_toml(manifest).as_deref(),
            Some("3.0.1")
        );
    }

    #[test]
    fn no_package_version_yields_none() {
        let manifest = r#"
[dependencies]
serde = { version = "1.0" }
"#;
        assert_eq!(package_version_from_cargo_toml(manifest), None);
    }
}
