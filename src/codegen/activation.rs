//! Generate Activation trait implementation and RPC server

use crate::parse::{
    Cardinality, ChildForward, ChildMethodInfo, ChildMethodKind, ListSearchMethodInfo, MethodInfo,
    ParsedDeprecation,
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
    // AUTH-3: default roles, flattened to grants.
    roles: &[(String, Vec<String>)],
    // MACRO-31: under `typed_params` a key no parameter claims is refused.
    typed_params: bool,
    // RETURNS-47: the trait of `impl Trait for Type`; its methods are called
    // through it (`<Self as Trait>::m(self, …)`), whose path need not be in
    // scope where the expansion lands.
    trait_impl: Option<&syn::Path>,
    // NEST-16: the activation names its parent: its schema says its needs.
    has_parent: bool,
) -> TokenStream {
    let enum_name = format_ident!("{}Method", struct_name);
    let rpc_trait_name = format_ident!("{}Rpc", struct_name);
    let rpc_server_name = format_ident!("{}RpcServer", struct_name);
    let method_names: Vec<&str> = methods.iter().map(|m| m.method_name.as_str()).collect();

    let dispatch_arms = generate_dispatch_arms(methods, namespace, crate_path, request_type, deps, typed_params, trait_impl);
    let help_arms = generate_help_arms(methods);
    // RETURNS-47: `impl Trait for Type` has no direct JSON-RPC methods —
    // they carry no call context (a trait method reaches its caller only
    // through it), and their server trait's methods, named as the trait's,
    // would make a call by method syntax ambiguous beside it. Every call goes
    // through the service's `{ns}.call`.
    let has_public_rpc_methods = trait_impl.is_none()
        && methods.iter().any(|m| !m.requires_auth && !m.takes_reach && m.activation_params.is_empty() && m.input.is_none() && m.parent.is_none());
    let rpc_trait_methods = generate_rpc_trait_methods(methods, deps);
    let rpc_impl_methods = generate_rpc_impl_methods(struct_name, methods, namespace, crate_path, deps, trait_impl);

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
                .and_then(|content| {
                    // Simple parse: find version = "x.y.z"
                    content.lines()
                        .find(|line| line.trim().starts_with("version"))
                        .and_then(|line| {
                            line.split('"').nth(1).map(|s| s.to_string())
                        })
                })
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

    // AUTH-3: `PluginSchema.roles` from `roles(...)`; nothing when none were
    // declared, so the schema is unchanged for every existing activation.
    let deprecation_inject = if roles.is_empty() {
        deprecation_inject
    } else {
        let role_names: Vec<&str> = roles.iter().map(|(n, _)| n.as_str()).collect();
        let role_grants: Vec<Vec<&str>> =
            roles.iter().map(|(_, g)| g.iter().map(String::as_str).collect()).collect();
        quote! {
            #deprecation_inject
            #(
                __plugin_schema.roles.insert(
                    ::std::string::String::from(#role_names),
                    ::std::vec![#(::std::string::String::from(#role_grants)),*],
                );
            )*
        }
    };

    // AUTH-3: under `namespace_fn` the grant follows the runtime namespace
    // (`menger.call`, not the macro's static `plexus.call`).
    let deprecation_inject = if namespace_fn.is_some() {
        quote! {
            #deprecation_inject
            {
                let __ns = ::std::string::ToString::to_string(self.namespace());
                for __m in __plugin_schema.methods.iter_mut() {
                    if __m.grant.is_some() {
                        __m.grant = ::std::option::Option::Some(::std::format!("{}.{}", __ns, __m.name));
                    }
                }
            }
        }
    } else {
        deprecation_inject
    };

    // NEST-16: `PluginSchema.needs`, for an activation that names its parent.
    let deprecation_inject = if has_parent {
        quote! {
            #deprecation_inject
            __plugin_schema.needs = ::std::option::Option::Some(Self::__plexus_needs());
        }
    } else {
        deprecation_inject
    };

    // NEST-16: the needs, answered without building the schema.
    let needs_fn = if has_parent {
        quote! {
            fn needs(&self) -> ::std::option::Option<::std::vec::Vec<#crate_path::plexus::Need>> {
                ::std::option::Option::Some(Self::__plexus_needs())
            }
        }
    } else {
        quote! {}
    };

    // NEST-5: the slot type's roles, as `roles(...)` declared them.
    let slot_roles_inject = {
        let role_names: Vec<&str> = roles.iter().map(|(n, _)| n.as_str()).collect();
        let role_grants: Vec<Vec<&str>> =
            roles.iter().map(|(_, g)| g.iter().map(String::as_str).collect()).collect();
        quote! {
            #(
                __plugin_schema.roles.insert(
                    ::std::string::String::from(#role_names),
                    ::std::vec![#(::std::string::String::from(#role_grants)),*],
                );
            )*
        }
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

    // PLEXUS-44: the server trait jsonrpsee's `#[rpc(server)]` would write,
    // written here. jsonrpsee's attribute looks itself up in the consumer's
    // Cargo.toml and fails when `jsonrpsee` is not listed there, which kept it
    // in every app's manifest; written by this macro it is named through the
    // shim like every other dependency, and the umbrella is enough.
    let rpc_server_trait = generate_rpc_server_trait(
        &rpc_trait_name,
        &rpc_server_name,
        &rpc_trait_methods,
        methods,
        namespace,
        deps,
    );
    let rpc_block = if has_public_rpc_methods {
        quote! {
            #rpc_server_trait

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
            .find(|c| matches!(c.kind, ChildMethodKind::Dynamic) && c.peer.is_none());

        let static_names: Vec<String> =
            static_children.iter().map(|c| c.fn_name.to_string()).collect();
        let static_calls: Vec<TokenStream> = static_children
            .iter()
            .map(|c| {
                let fn_name = &c.fn_name;
                // Static child methods have shape `fn NAME(&self) -> Child`
                // which we always call synchronously. (Making static child
                // methods async would just force an unnecessary `.await`.)
                quote! {
                    Some(Box::new(self.#fn_name()) as Box<dyn #crate_path::plexus::ChildRouter>)
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
                    .map(|c| Box::new(c) as Box<dyn #crate_path::plexus::ChildRouter>),
            }
        } else {
            quote! { _ => None, }
        };

        if static_children.is_empty() && dynamic_child.is_none() {
            // NEST-7: only peer slots (routed by `peer_child` below).
            quote! { None }
        } else if static_children.is_empty() {
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
                    .map(|c| Box::new(c) as Box<dyn #crate_path::plexus::ChildRouter>)
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
                    #child_names => Some(Box::new(self.#child_fields.clone())),
                )*
                _ => None,
            }
        }
    };

    // NEST-7: `#[peer]` slots, each reached by its name
    // (`host.integration.github.repos`). Routing asks `peer_child`, which
    // answers a static child and the dynamic `#[child]` as `get_child`
    // would, and for a slot's name a `PeerSlot`: its children are the peers
    // the slot's function returns (a typed one checked against the slot's
    // interface), each failure the caller's typed error item.
    let peer_slots: Vec<&ChildMethodInfo> = child_methods.iter().filter(|c| c.peer.is_some()).collect();
    let (get_child_body, peer_child_impl) = if peer_slots.is_empty() {
        (get_child_body, quote! {})
    } else {
        let statics: Vec<&ChildMethodInfo> = child_methods.iter().filter(|c| matches!(c.kind, ChildMethodKind::Static)).collect();
        let static_names: Vec<String> = statics.iter().map(|c| c.fn_name.to_string()).collect();
        let static_fns: Vec<&syn::Ident> = statics.iter().map(|c| &c.fn_name).collect();
        let slot_names: Vec<String> = peer_slots.iter().map(|c| c.fn_name.to_string()).collect();
        let slot_arms: Vec<TokenStream> = peer_slots
            .iter()
            .map(|c| {
                let fn_name = &c.fn_name;
                let name = fn_name.to_string();
                let interface = crate::codegen::peer::interface_expr(c.peer.as_ref().unwrap(), crate_path);
                let with_list = match &c.list_fn {
                    Some(list) => {
                        let await_tok = if c.list_is_async { quote! { .await } } else { quote! {} };
                        quote! {
                            .with_list({
                                let __this = ::core::clone::Clone::clone(self);
                                ::std::sync::Arc::new(move || {
                                    let __this = ::core::clone::Clone::clone(&__this);
                                    ::std::boxed::Box::pin(async move {
                                        let __names = __this.#list()#await_tok;
                                        #deps::futures::StreamExt::collect::<::std::vec::Vec<::std::string::String>>(__names).await
                                    })
                                })
                            })
                        }
                    }
                    None => quote! {},
                };
                quote! {
                    {
                        let __this = ::core::clone::Clone::clone(self);
                        let __find: #crate_path::plexus::peer::FindPeer = ::std::sync::Arc::new(move |__name: ::std::string::String| {
                            let __this = ::core::clone::Clone::clone(&__this);
                            ::std::boxed::Box::pin(async move {
                                __this.#fn_name(&__name).await.map(|__p| {
                                    let __p: ::std::boxed::Box<dyn #crate_path::plexus::peer::Peer> = __p;
                                    __p
                                })
                            })
                        });
                        ::core::result::Result::Ok(::core::option::Option::Some(::std::boxed::Box::new(
                            #crate_path::plexus::peer::PeerSlot::new(#name, #interface, __find)#with_list,
                        ) as Box<dyn #crate_path::plexus::ChildRouter>))
                    }
                }
            })
            .collect();
        let dynamic_arm = match child_methods.iter().find(|c| matches!(c.kind, ChildMethodKind::Dynamic) && c.peer.is_none()) {
            Some(d) => {
                let fn_name = &d.fn_name;
                let await_tok = if d.is_async { quote! { .await } } else { quote! {} };
                quote! {
                    _ => ::core::result::Result::Ok(self.#fn_name(_name)#await_tok.map(|c| Box::new(c) as Box<dyn #crate_path::plexus::ChildRouter>)),
                }
            }
            None => quote! { _ => ::core::result::Result::Ok(::core::option::Option::None), },
        };
        (
            quote! {
                <Self as #crate_path::plexus::ChildRouter>::peer_child(self, _name).await.ok().flatten()
            },
            quote! {
                async fn peer_child(
                    &self,
                    _name: &str,
                ) -> ::core::result::Result<
                    ::core::option::Option<Box<dyn #crate_path::plexus::ChildRouter>>,
                    #crate_path::plexus::peer::PeerError,
                > {
                    match _name {
                        #(
                            #static_names => ::core::result::Result::Ok(::core::option::Option::Some(
                                Box::new(self.#static_fns()) as Box<dyn #crate_path::plexus::ChildRouter>,
                            )),
                        )*
                        #( #slot_names => #slot_arms, )*
                        #dynamic_arm
                    }
                }
            },
        )
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

    // MACRO-20: how each child inherits its caller, and what the parent can
    // say about its static children's schemas.
    //
    // `forward_policy_for`: a static child is the same service as its parent
    // (a value built from the parent's own fields, in-process): it gets
    // `SameService` — principal stamp, session, ceiling — unless its
    // `#[child(forward = "...")]` says otherwise. A dynamic child keeps the
    // framework default (`IdentityOnly`) unless it names a policy. No
    // `#[child]` at all: the trait default, unchanged.
    let forward_policy_impl = if child_methods.is_empty() {
        quote! {}
    } else {
        let policy = |f: ChildForward| -> TokenStream {
            let ty = match f {
                ChildForward::SameService => quote! { #crate_path::plexus::SameService },
                ChildForward::IdentityOnly => quote! { #crate_path::plexus::IdentityOnly },
                ChildForward::PassThrough => quote! { #crate_path::plexus::PassThrough },
                ChildForward::Anonymous => quote! { #crate_path::plexus::Anonymous },
            };
            quote! {
                ::std::option::Option::Some(
                    ::std::sync::Arc::new(#ty) as ::std::sync::Arc<dyn #crate_path::plexus::ForwardPolicy>
                )
            }
        };
        let static_arms: Vec<TokenStream> = child_methods
            .iter()
            .filter(|c| matches!(c.kind, ChildMethodKind::Static))
            .map(|c| {
                let name = c.fn_name.to_string();
                let p = policy(c.forward.unwrap_or(ChildForward::SameService));
                quote! { #name => #p, }
            })
            .collect();
        let default_arm = match child_methods
            .iter()
            .find(|c| matches!(c.kind, ChildMethodKind::Dynamic) && c.peer.is_none())
            .and_then(|c| c.forward)
        {
            Some(f) => {
                let p = policy(f);
                quote! { _ => #p, }
            }
            None => quote! { _ => ::std::option::Option::None, },
        };
        quote! {
            // MACRO-20: a static child inherits its caller as the same
            // service does (SameService) unless `#[child(forward = "...")]`
            // says otherwise.
            fn forward_policy_for(
                &self,
                callee_ns: &str,
            ) -> ::std::option::Option<::std::sync::Arc<dyn #crate_path::plexus::ForwardPolicy>> {
                match callee_ns {
                    #(#static_arms)*
                    #default_arm
                }
            }
        }
    };

    // `router_static_child_schemas`: each static child's own schema (when it
    // describes itself) and its static children under its name — what a hub
    // gates a child's method by (`<activation>.<child>.<method>`).
    let static_child_fns: Vec<&syn::Ident> = child_methods
        .iter()
        .filter(|c| matches!(c.kind, ChildMethodKind::Static))
        .map(|c| &c.fn_name)
        .collect();
    let static_child_strs: Vec<String> = static_child_fns.iter().map(|f| f.to_string()).collect();
    let static_child_schemas_impl = if static_child_fns.is_empty() {
        quote! {}
    } else {
        quote! {
            fn router_static_child_schemas(
                &self,
            ) -> ::std::vec::Vec<(::std::string::String, #crate_path::plexus::PluginSchema)> {
                let mut out = ::std::vec::Vec::new();
                #({
                    let child = self.#static_child_fns();
                    if let ::std::option::Option::Some(schema) =
                        <_ as #crate_path::plexus::ChildRouter>::router_schema(&child)
                    {
                        out.push((::std::string::String::from(#static_child_strs), schema));
                    }
                    for (path, schema) in <_ as #crate_path::plexus::ChildRouter>::router_static_child_schemas(&child) {
                        out.push((::std::format!("{}.{}", #static_child_strs, path), schema));
                    }
                })*
                out
            }
        }
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

                #peer_child_impl

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

                #forward_policy_impl

                // MACRO-20: this activation describes itself to its parent.
                fn router_schema(&self) -> ::std::option::Option<#crate_path::plexus::PluginSchema> {
                    ::std::option::Option::Some(<Self as #crate_path::plexus::Activation>::plugin_schema(self))
                }

                #static_child_schemas_impl
            }
        }
    };

    // MACRO-20: the hub's registry holds `Activation`s; it reaches the static
    // children through this (the same list as the ChildRouter method).
    let activation_static_children_impl = if static_child_fns.is_empty() || (hub_explicit && child_methods.is_empty()) {
        quote! {}
    } else {
        quote! {
            fn static_child_schemas(
                &self,
            ) -> ::std::vec::Vec<(::std::string::String, #crate_path::plexus::PluginSchema)> {
                <Self as #crate_path::plexus::ChildRouter>::router_static_child_schemas(self)
            }
        }
    };

    // CHILD-8: synthesize `plugin_children(&self) -> Vec<ChildSummary>` from
    // the static `#[child]` methods when the user hasn't defined one. Dynamic
    // `#[child]` methods are intentionally omitted — an open-ended name set
    // has no meaningful finite summary list; dynamic children are discoverable
    // via `ChildRouter::list_children` at runtime.
    //
    // RETURNS-25: each `ChildSummary.hash` is the child's own schema hash —
    // `Activation::plugin_schema(&self.child()).hash`, computed at runtime the
    // way DynamicHub's `plugin_children` does it (it used to be `""`, so a
    // change inside a static child reached no ancestor's hash). It rolls up
    // into this service's `children_hash` and `hash`, so a change anywhere below
    // moves every ancestor's hash and the root's. A static child that is not
    // an `Activation` — a hand-written `ChildRouter` (MACRO-20) — describes
    // nothing, and its hash is `""` as before. The child is
    // built by its accessor and its schema computed on each call; a type that
    // is its own static descendant would recurse without end.
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
                let fn_name = &c.fn_name;
                quote! {
                    #crate_path::plexus::ChildSummary {
                        namespace: ::std::string::String::from(#name),
                        description: ::std::string::String::from(#desc),
                        // RETURNS-36: by autoref, the child's schema hash
                        // when it is an `Activation` (RETURNS-25), and `""`
                        // when it is a hand-written `ChildRouter` that
                        // describes nothing (MACRO-20 still compiles it).
                        hash: {
                            struct __PlexusChildHash<'a, C>(&'a C);
                            trait __PlexusViaActivation {
                                fn __plexus_child_hash(&self) -> ::std::string::String;
                            }
                            impl<C: #crate_path::plexus::Activation> __PlexusViaActivation for __PlexusChildHash<'_, C> {
                                fn __plexus_child_hash(&self) -> ::std::string::String {
                                    #crate_path::plexus::Activation::plugin_schema(self.0).hash
                                }
                            }
                            trait __PlexusViaRouter {
                                fn __plexus_child_hash(&self) -> ::std::string::String;
                            }
                            impl<C> __PlexusViaRouter for &__PlexusChildHash<'_, C> {
                                fn __plexus_child_hash(&self) -> ::std::string::String {
                                    ::std::string::String::new()
                                }
                            }
                            let __child = self.#fn_name();
                            (&__PlexusChildHash(&__child)).__plexus_child_hash()
                        },
                    }
                }
            })
            .collect();
        // NEST-7: each `#[peer]` slot is a child by its own name
        // (`host.integration`), whose schema lists the slot's names.
        let peer_entries: Vec<TokenStream> = child_methods
            .iter()
            .filter(|c| c.peer.is_some())
            .map(|c| {
                let name = c.fn_name.to_string();
                let desc = &c.description;
                // The slot's own hash (its interface's, and its name), never
                // a peer's: a navigator checks for cycles by hash.
                let hash = match c.peer.as_ref().unwrap() {
                    crate::codegen::peer::PeerShape::Typed(path) => quote! {
                        #crate_path::plexus::content_hash::hash16(
                            ::std::format!("slot:{}:{}", #name, <dyn #path as #crate_path::plexus::peer::Contract>::interface().hash).as_bytes(),
                        )
                    },
                    crate::codegen::peer::PeerShape::Untyped => quote! {
                        #crate_path::plexus::content_hash::hash16(::std::format!("slot:{}", #name).as_bytes())
                    },
                };
                quote! {
                    #crate_path::plexus::ChildSummary {
                        namespace: ::std::string::String::from(#name),
                        description: ::std::string::String::from(#desc),
                        hash: #hash,
                    }
                }
            })
            .collect();
        let entries: Vec<TokenStream> = entries.into_iter().chain(peer_entries).collect();
        quote! {
            /// Auto-generated child summary list from `#[plexus_macros::child]`
            /// static methods, each with its child's schema hash. Dynamic
            /// children are not listed here; they are enumerable at runtime
            /// via `ChildRouter::list_children`.
            pub fn plugin_children(&self) -> ::std::vec::Vec<#crate_path::plexus::ChildSummary> {
                ::std::vec![
                    #(#entries,)*
                ]
            }
        }
    } else {
        quote! {}
    };

    quote! {
        impl #impl_generics #self_ty #where_clause {
            pub const NAMESPACE: &'static str = #namespace;
            /// Stable plugin instance ID for handle routing
            pub const PLUGIN_ID: #deps::uuid::Uuid = #deps::uuid::uuid!(#plugin_id_str);

            /// Get the plugin's stable instance ID
            pub fn plugin_id(&self) -> #deps::uuid::Uuid {
                Self::PLUGIN_ID
            }

            #plugin_children_synth
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

            async fn call(
                &self,
                method: &str,
                params: #deps::serde_json::Value,
                auth: Option<&#crate_path::plexus::AuthContext>,
                raw_ctx: Option<&#crate_path::request::RawRequestContext>,
            ) -> Result<#crate_path::plexus::PlexusStream, #crate_path::plexus::PlexusError> {
                // Try local methods first
                match method {
                    #(#dispatch_arms)*
                    "schema" => {
                        // Check if a specific method was requested
                        let method_name: Option<String> = params.get("method")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());

                        let plugin_schema = self.plugin_schema();

                        let result = if let Some(ref name) = method_name {
                            // Find the specific method
                            plugin_schema.methods.iter()
                                .find(|m| m.name == *name)
                                .map(|m| #crate_path::plexus::SchemaResult::Method(m.clone()))
                                .ok_or_else(|| #crate_path::plexus::PlexusError::MethodNotFound {
                                    activation: #namespace.to_string(),
                                    method: name.clone(),
                                })?
                        } else {
                            // Return full plugin schema
                            #crate_path::plexus::SchemaResult::Plugin(plugin_schema)
                        };

                        Ok(#crate_path::plexus::wrap_stream(
                            #deps::futures::stream::once(async move { result }),
                            concat!(#namespace, ".schema"),
                            vec![#namespace.into()]
                        ))
                    }
                    _ => {
                        // Check for {method}.schema pattern (e.g., "echo.schema")
                        // Only if the prefix is an actual local user method (NOT a
                        // child-role entry). Child-role methods are listed in
                        // `plugin_schema.methods` with `role = StaticChild` or
                        // `DynamicChild { .. }` (CHILD-8 / IR-10); matching them
                        // here would return a `SchemaResult::Method` when the
                        // caller actually wants the child's `PluginSchema` via
                        // `route_to_child` (HF-AUDIT-3 / HF-AUDIT-4).
                        if let Some(method_name) = method.strip_suffix(".schema") {
                            let plugin_schema = self.plugin_schema();
                            if let Some(m) = plugin_schema.methods.iter().find(|m| {
                                m.name == method_name
                                    && !matches!(
                                        m.role,
                                        #crate_path::plexus::MethodRole::StaticChild
                                            | #crate_path::plexus::MethodRole::DynamicChild { .. }
                                            | #crate_path::plexus::MethodRole::Peer { .. }
                                    )
                            }) {
                                let result = #crate_path::plexus::SchemaResult::Method(m.clone());
                                return Ok(#crate_path::plexus::wrap_stream(
                                    #deps::futures::stream::once(async move { result }),
                                    concat!(#namespace, ".method_schema"),
                                    vec![#namespace.into()]
                                ));
                            }
                            // Not a local user method (child-role entry or no
                            // match) - fall through to child routing.
                        }

                        // For hubs: try routing to child plugin via ChildRouter trait
                        // For leaves: return MethodNotFound
                        #call_fallback
                    }
                }
            }

            fn into_rpc_methods(self) -> #deps::jsonrpsee::core::server::Methods {
                #into_rpc_body
            }

            fn plugin_schema(&self) -> #crate_path::plexus::PluginSchema {
                #plugin_schema_body
            }

            #needs_fn

            // NEST-5: the schema without an instance — a parent's slot type.
            fn slot_schema() -> ::std::option::Option<#crate_path::plexus::PluginSchema>
            where
                Self: ::core::marker::Sized,
            {
                let mut __plugin_schema = #crate_path::plexus::PluginSchema::leaf(
                    #namespace,
                    #version_expr,
                    #description,
                    #enum_name::method_schemas(),
                );
                #slot_roles_inject
                ::std::option::Option::Some(__plugin_schema)
            }

            #activation_static_children_impl

            #resolve_handle_impl
        }
    }
}

/// The methods the jsonrpsee server trait carries: those `Activation::call`
/// need not handle itself.
fn rpc_methods(methods: &[MethodInfo]) -> impl Iterator<Item = &MethodInfo> {
    methods
        .iter()
        // Auth-requiring methods and methods with activation_params are dispatched through
        // Activation::call() which handles auth injection and request extraction.
        // The jsonrpsee RPC trait cannot carry auth context or raw_ctx.
        .filter(|m| !m.requires_auth && !m.takes_reach && m.activation_params.is_empty() && m.input.is_none() && m.parent.is_none())
}

/// The server trait's method signatures, as jsonrpsee's `#[rpc(server)]`
/// wrote them from a `#[subscription]`: the sink second.
fn generate_rpc_trait_methods(methods: &[MethodInfo], deps: &syn::Ident) -> Vec<TokenStream> {
    rpc_methods(methods)
        .map(|m| {
            let method_name = &m.fn_name;
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
                async fn #method_name(
                    &self,
                    subscription_sink: #deps::jsonrpsee::PendingSubscriptionSink,
                    #(#params),*
                ) -> #deps::jsonrpsee::core::SubscriptionResult;
            }
        })
        .collect()
}

/// PLEXUS-44: `{Struct}RpcServer`, token for token what jsonrpsee 0.26's
/// `#[rpc(server, namespace = "ns")]` rendered for a trait of async
/// `#[subscription(name = "m", unsubscribe = "unsubscribe_m")]` methods
/// (`render_server.rs`): the same trait, the same `into_rpc`, the same names
/// registered (`ns_m`, notifications as `ns_m`, `ns_unsubscribe_m`) and the
/// same parameter decoding — a JSON object by name (each name also accepted
/// in snake_case and lowerCamelCase), anything else by position, an
/// `Option` parameter optional; a parameter that fails to decode rejects the
/// subscription with jsonrpsee's own error. Only the spelling of the
/// dependency paths differs: through the shim, so no consumer lists
/// `jsonrpsee`.
fn generate_rpc_server_trait(
    rpc_trait_name: &syn::Ident,
    rpc_server_name: &syn::Ident,
    trait_methods: &[TokenStream],
    methods: &[MethodInfo],
    namespace: &str,
    deps: &syn::Ident,
) -> TokenStream {
    // A derive's `crate = "..."` resolves module-relatively and cannot see
    // the shim when the activation is declared in a fn body (see
    // method_enum.rs), so the decoding struct's serde is named absolutely:
    // through the umbrella, or through jsonrpsee as jsonrpsee's macro did.
    let (serde_path, serde_crate) = match crate::codegen::umbrella_crate(proc_macro2::Span::call_site()) {
        Some(u) => (quote! { ::#u::__deps::serde }, format!("::{}::__deps::serde", u)),
        None => (
            quote! { ::jsonrpsee::core::__reexports::serde },
            "::jsonrpsee::core::__reexports::serde".to_string(),
        ),
    };
    let reexports = quote! { #deps::jsonrpsee::core::__reexports };

    let registrations: Vec<TokenStream> = rpc_methods(methods)
        .map(|m| {
            let rust_method_name = &m.fn_name;
            let sub_name = format!("{}_{}", namespace, m.method_name);
            let unsub_name = format!("{}_unsubscribe_{}", namespace, m.method_name);

            let error_ret = quote! {
                #reexports::tokio::spawn(pending.reject(e));
                return #deps::jsonrpsee::SubscriptionCloseResponse::None;
            };

            let (params_ident, parsing, params_seq) = if m.params.is_empty() {
                (quote! { _ }, quote! {}, quote! {})
            } else {
                let pats: Vec<&syn::Ident> = m.params.iter().map(|p| &p.name).collect();
                let fields = quote! { #(#pats),* };

                let decode_fields = m.params.iter().map(|p| {
                    let pat = &p.name;
                    let ty = &p.ty;
                    let is_option = is_option(ty);
                    let next = if is_option { quote!(optional_next) } else { quote!(next) };
                    quote! {
                        let #pat: #ty = match seq.#next() {
                            Ok(v) => v,
                            Err(e) => {
                                #reexports::log_fail_parse(stringify!(#pat), stringify!(#ty), &e, #is_option);
                                #error_ret
                            }
                        };
                    }
                });

                let generics: Vec<syn::Ident> = (0..m.params.len()).map(|n| format_ident!("G{}", n)).collect();
                let object_fields = m.params.iter().zip(generics.iter()).map(|(p, g)| {
                    let pat = &p.name;
                    let name = p.name.to_string();
                    let snake = heck::ToSnakeCase::to_snake_case(name.as_str());
                    let camel = heck::ToLowerCamelCase::to_lower_camel_case(name.as_str());
                    quote! {
                        #[serde(alias = #snake, alias = #camel)]
                        #[serde(rename = #name)]
                        #pat: #g,
                    }
                });
                let destruct = pats.iter().map(|p| quote! { parsed.#p });
                let types = m.params.iter().map(|p| &p.ty);

                let parsing = quote! {
                    let (#fields) = if params.is_object() {
                        #[derive(#serde_path::Deserialize)]
                        #[serde(crate = #serde_crate)]
                        struct ParamsObject<#(#generics,)*> {
                            #(#object_fields)*
                        }

                        let parsed: ParamsObject<#(#types,)*> = match params.parse() {
                            Ok(p) => p,
                            Err(e) => {
                                #reexports::log_fail_parse_as_object(&e);
                                #error_ret
                            }
                        };

                        (#(#destruct),*)
                    } else {
                        let mut seq = params.sequence();
                        #(#decode_fields)*
                        (#fields)
                    };
                };
                (quote! { params }, parsing, quote! { #fields })
            };

            quote! {{
                let _res = rpc.register_subscription(
                    #sub_name,
                    #sub_name,
                    #unsub_name,
                    |#params_ident, pending, context, _| async move {
                        #parsing
                        #deps::jsonrpsee::IntoSubscriptionCloseResponse::into_response(
                            context.as_ref().#rust_method_name(pending, #params_seq).await,
                        )
                    },
                );
                #[cfg(debug_assertions)]
                if _res.is_err() {
                    #reexports::panic_fail_register();
                }
            }}
        })
        .collect();

    let doc = format!("Server trait implementation for the `{}` RPC API.", rpc_trait_name);
    quote! {
        #[#deps::async_trait::async_trait]
        #[doc = #doc]
        pub trait #rpc_server_name: Sized + Send + Sync + 'static {
            #(#trait_methods)*

            #[doc = "Collects all the methods and subscriptions defined in the trait and adds them into a single `RpcModule`."]
            fn into_rpc(self) -> #deps::jsonrpsee::RpcModule<Self> {
                let mut rpc = #deps::jsonrpsee::RpcModule::new(self);
                #(#registrations)*
                rpc
            }
        }
    }
}

/// `Option<..>` as jsonrpsee's macro recognised it: a last segment `Option`.
fn is_option(ty: &syn::Type) -> bool {
    if let syn::Type::Path(path) = ty {
        let mut it = path.path.segments.iter().peekable();
        while let Some(seg) = it.next() {
            if seg.ident == "Option" && it.peek().is_none() {
                return true;
            }
        }
    }
    false
}

fn generate_rpc_impl_methods(
    struct_name: &syn::Ident,
    methods: &[MethodInfo],
    namespace: &str,
    crate_path: &syn::Path,
    deps: &syn::Ident,
    trait_impl: Option<&syn::Path>,
) -> Vec<TokenStream> {
    // RETURNS-47: an inherent method by the type's path, a trait's through
    // the trait.
    let recv = match trait_impl {
        None => quote! { #struct_name },
        Some(t) => quote! { <Self as #t> },
    };

    methods
        .iter()
        // Skip auth-requiring methods and activation_param methods — they route through Activation::call()
        .filter(|m| !m.requires_auth && !m.takes_reach && m.activation_params.is_empty() && m.input.is_none() && m.parent.is_none())
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

            let method_call = quote! { #recv::#method_name(self, #(#param_names),*).await };

            // RETURNS-8: the same per-form wrapper as `Activation::call`, so
            // both paths send the same frames.
            let content_type = format!("{}.{}", namespace, method_name_str);
            let wrapped = wrap_return(m, quote! { __ret }, &content_type, namespace, crate_path);

            quote! {
                async fn #method_name(
                    &self,
                    pending: #deps::jsonrpsee::PendingSubscriptionSink,
                    #(#params_with_types),*
                ) -> #deps::jsonrpsee::core::SubscriptionResult {
                    use #deps::futures::StreamExt;

                    let sink = pending.accept().await?;
                    let __ret = #method_call;
                    let wrapped = #wrapped;

                    // The wrapper ends the stream with its one `done`
                    // (RETURNS-8: this task used to append a second).
                    #deps::tokio::spawn(async move {
                        let mut stream = wrapped;
                        while let Some(item) = stream.next().await {
                            if let Ok(raw_value) = #deps::serde_json::value::to_raw_value(&item) {
                                if sink.send(raw_value).await.is_err() {
                                    break;
                                }
                            }
                        }
                    });

                    Ok(())
                }
            }
        })
        .collect()
}

fn generate_dispatch_arms(
    methods: &[MethodInfo],
    namespace: &str,
    crate_path: &syn::Path,
    request_type: Option<&syn::Type>,
    deps: &syn::Ident,
    typed_params: bool,
    trait_impl: Option<&syn::Path>,
) -> Vec<TokenStream> {
    // RETURNS-47: how the dispatch calls a method — by method syntax on an
    // inherent impl, through the trait on `impl Trait for Type`.
    let invoke = |fn_name: &syn::Ident, args: TokenStream| match trait_impl {
        None => quote! { self.#fn_name(#args) },
        Some(t) => quote! { <Self as #t>::#fn_name(self, #args) },
    };

    methods
        .iter()
        .map(|m| {
            let method_name = &m.method_name;
            let fn_name = &m.fn_name;

            let content_type = format!("{}.{}", namespace, method_name);

            // Generate param extraction code (MACRO-31: under `typed_params`,
            // preceded by the check for keys no parameter claims).
            let (param_extraction, mut param_names) = generate_param_extraction(m, crate_path, deps);
            // BIDIR-6: the input half opens last, after the params and the
            // gate, right before the method runs; it takes its place among
            // the arguments as it does in the signature.
            // NEST-16: the injected parent takes its place after the input's.
            let parent_binding = crate::codegen::parent::binding(m, namespace, crate_path);
            let input_open = match &m.input {
                Some(i) => {
                    param_names.insert(i.at, &i.name);
                    let (name, item) = (&i.name, &i.item);
                    let ceiling = input_ceiling(i, crate_path);
                    quote! {
                        let #name = #crate_path::plexus::input::open::<#item>(#content_type, #ceiling).await?;
                    }
                }
                None => quote! {},
            };
            if let Some(p) = &m.parent {
                param_names.insert(p.at, &p.name);
            }
            let unknown_keys = if typed_params {
                generate_unknown_key_check(m, &content_type, crate_path, deps)
            } else {
                quote! {}
            };
            let param_extraction = quote! { #unknown_keys #param_extraction };

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
            //
            // MACRO-14: the resolver's error is converted, not stringified.
            // A `PlexusError` (a typed `Forbidden` / `Unauthenticated`)
            // passes through unchanged, so it reaches the wire with its code
            // and `error_kind`; any other `Display` error is
            // `ExecutionError(e.to_string())`, as before. The choice is made
            // by autoref dispatch on the error's concrete type: the
            // by-reference impl for `__PlexusResolverErr<PlexusError>` is
            // found one autoref step before the blanket `Display` impl on
            // `&__PlexusResolverErr<E>`, so no existing resolver changes.
            let resolver_bindings: Vec<proc_macro2::TokenStream> = m.auth_resolvers.iter().map(|r| {
                resolve_typed(&r.param_name, &r.resolver_expr, crate_path)
            }).collect();

            // RETURNS-47: a trait-impl method that needs its caller reads it
            // from the call's context — outside one, or anonymous, refused
            // here, before the body, typed, as `auth.ok_or(Unauthenticated)`
            // refuses for `#[from_auth]` — and runs its verifier as a
            // `#[from_auth]` resolver runs (the same typed error route),
            // keeping the value for `plexus_core::call::verified`.
            let caller_gate = if m.caller_required {
                let verify = m.verifier.as_ref().map(|expr| {
                    let name = format_ident!("__plexus_verified");
                    let bind = resolve_typed(&name, expr, crate_path);
                    quote! {
                        #bind
                        #crate_path::call::__keep_verified(#name)?;
                    }
                });
                quote! {
                    let __plexus_caller = #crate_path::call::caller()?;
                    let auth_ctx = __plexus_caller.signed_in()?;
                    #verify
                }
            } else {
                quote! {}
            };

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

            // AUTH-2: `reach: Reach` — the gate's answer for this call, read
            // from the call's context (RETURNS-47: everywhere only when the
            // dispatch said so, a service with no gate; outside a call the
            // method is refused, never handed everything). Passed first, or
            // right after `auth`.
            let reach_binding = if m.takes_reach {
                quote! { let __plexus_reach = #crate_path::plexus::current_reach()?; }
            } else {
                quote! {}
            };
            let reach_arg = if m.takes_reach { quote! { __plexus_reach, } } else { quote! {} };

            // Build parameter list for method call
            let activation_param_names: Vec<&syn::Ident> = m.activation_params.iter().map(|ap| &ap.param_name).collect();
            let resolver_names: Vec<&syn::Ident> = m.auth_resolvers.iter().map(|r| &r.param_name).collect();
            let method_params = if m.requires_auth && m.auth_resolvers.is_empty() {
                // Raw auth context pass-through (legacy)
                quote! { auth_ctx, #reach_arg #(#param_names),* }
            } else if !m.activation_params.is_empty() && !m.auth_resolvers.is_empty() {
                // Both activation_params and resolver params + regular params
                quote! { #reach_arg #(#activation_param_names,)* #(#resolver_names,)* #(#param_names),* }
            } else if !m.activation_params.is_empty() {
                // activation_params + regular params
                quote! { #reach_arg #(#activation_param_names,)* #(#param_names),* }
            } else if !m.auth_resolvers.is_empty() {
                // Resolved parameters + regular params
                quote! { #reach_arg #(#resolver_names,)* #(#param_names),* }
            } else {
                quote! { #reach_arg #(#param_names),* }
            };

            let call = invoke(fn_name, method_params.clone());

            // RETURNS-8: the wrapper that matches the method's signature
            // form turns its return value into frames.
            let wrapped = wrap_return(m, quote! { __ret }, &content_type, namespace, crate_path);

            quote! {
                #method_name => {
                    #auth_injection
                    #activation_extraction
                    #(#activation_param_bindings)*
                    #caller_gate
                    #(#resolver_bindings)*
                    #param_extraction
                    #reach_binding
                    #parent_binding
                    #input_open
                    let __ret = #call.await;
                    Ok(#wrapped)
                }
            }
        })
        .collect()
}

/// BIDIR-6: the `CallCeiling` a method's attributes say.
fn input_ceiling(i: &crate::parse::InputInfo, crate_path: &syn::Path) -> TokenStream {
    let set = |field: &str, v: &Option<syn::LitInt>| {
        let field = quote::format_ident!("{}", field);
        v.as_ref().map(|v| quote! { __c.#field = #v; })
    };
    let sets = [
        set("credit", &i.ceiling.window),
        set("max_items", &i.ceiling.max_items),
        set("max_bytes", &i.ceiling.max_bytes),
        set("max_buffered", &i.ceiling.max_buffered),
    ];
    quote! {{
        #[allow(unused_mut)]
        let mut __c = <#crate_path::plexus::duplex_gate::CallCeiling as ::core::default::Default>::default();
        #(#sets)*
        __c
    }}
}

/// MACRO-31: under `typed_params`, a key of the params object that names no
/// wire parameter of the method is `InvalidParams` (-32602) before anything
/// is extracted — ``unknown field `sumary`; tickets.list takes: after, …``,
/// every unknown key named and sorted, the parameters sorted (``… takes no
/// parameters`` for a method of none). `unknown field` is the words synapse
/// reads to know its cached schema is stale, so they are said verbatim. The
/// method is named by this activation's namespace (a child's own, not the
/// path it is mounted at: the dispatch is not handed that). Only an object
/// is looked at: `null`, a bare value and `{}` reach the extraction as
/// before, and keys inside a parameter's value are its type's to judge.
/// `m.params` holds the wire parameters alone — what the server injects
/// (`auth`, `reach`, `#[from_auth]`, `#[activation_param]`) is never a key.
fn generate_unknown_key_check(m: &MethodInfo, content_type: &str, crate_path: &syn::Path, deps: &syn::Ident) -> TokenStream {
    let mut known: Vec<String> = m.params.iter().map(|p| p.name.to_string()).collect();
    known.sort();
    let takes = if known.is_empty() {
        format!("{} takes no parameters", content_type)
    } else {
        format!("{} takes: {}", content_type, known.join(", "))
    };
    quote! {
        if let #deps::serde_json::Value::Object(__plexus_keys) = &params {
            const __PLEXUS_KNOWN: &[&str] = &[#(#known),*];
            let mut __plexus_unknown: ::std::vec::Vec<&str> = __plexus_keys
                .keys()
                .map(|k| k.as_str())
                .filter(|k| !__PLEXUS_KNOWN.contains(k))
                .collect();
            if !__plexus_unknown.is_empty() {
                __plexus_unknown.sort_unstable();
                let __plexus_named: ::std::vec::Vec<::std::string::String> =
                    __plexus_unknown.iter().map(|k| format!("`{}`", k)).collect();
                return Err(#crate_path::plexus::PlexusError::InvalidParams {
                    param: None,
                    problem: #crate_path::plexus::ParamProblem::Malformed {
                        detail: format!("unknown field {}; {}", __plexus_named.join(", "), #takes),
                    },
                });
            }
        }
    }
}

/// RETURNS-8: the expression that turns a method's return value `ret` into a
/// `PlexusStream`, by signature form (plexus-core's per-form wrappers):
///
/// | form | wrapper |
/// |---|---|
/// | `-> T` | `wrap_one` |
/// | `-> Result<T, E>` | `wrap_one_result` |
/// | `-> impl Stream<Item = T>` | `wrap_stream` |
/// | `-> impl Stream<Item = Result<T, E>>` | `wrap_result_stream` |
/// | `-> impl Stream<Item = Progress<T, P>>` | `wrap_progress_stream` |
/// | `-> impl Stream<Item = Result<Progress<T, P>, E>>` | `wrap_result_progress_stream` |
fn wrap_return(
    m: &MethodInfo,
    ret: TokenStream,
    content_type: &str,
    namespace: &str,
    crate_path: &syn::Path,
) -> TokenStream {
    let wrapper = match (m.cardinality, m.err_ty.is_some(), m.progress_ty.is_some()) {
        (Cardinality::One, false, _) => format_ident!("wrap_one"),
        (Cardinality::One, true, _) => format_ident!("wrap_one_result"),
        (Cardinality::Many, false, false) => format_ident!("wrap_stream"),
        (Cardinality::Many, true, false) => format_ident!("wrap_result_stream"),
        (Cardinality::Many, false, true) => format_ident!("wrap_progress_stream"),
        (Cardinality::Many, true, true) => format_ident!("wrap_result_progress_stream"),
    };
    quote! {
        #crate_path::plexus::#wrapper(#ret, #content_type, vec![#namespace.into()])
    }
}

/// Generate param extraction code and return the param names for the call.
///
/// MACRO-15: a parameter that does not deserialize is `InvalidParams`
/// naming it — `id: <serde's message>` — on every path (one param or many,
/// `Option` or required, the bare-value shortcut), so a method with two
/// parameters of one type says which was bad. RETURNS-6: structured —
/// `PlexusError::InvalidParams { param, problem }`, `Missing` when a
/// required param is absent, `Malformed { detail }` when one does not
/// decode, naming the param either way (`param: None` only when the params
/// value is not an object at all).
fn generate_param_extraction<'a>(m: &'a MethodInfo, crate_path: &syn::Path, deps: &syn::Ident) -> (TokenStream, Vec<&'a syn::Ident>) {
    let param_names: Vec<&syn::Ident> = m.params.iter().map(|p| &p.name).collect();

    // `|e| InvalidParams { param: Some(name), problem: Malformed { detail } }`
    let malformed = |name: &str| {
        quote! {
            |e| #crate_path::plexus::PlexusError::InvalidParams {
                param: Some(#name.to_string()),
                problem: #crate_path::plexus::ParamProblem::Malformed { detail: e.to_string() },
            }
        }
    };
    // `InvalidParams { param: Some(name), problem: Missing }`
    let missing = |name: &str| {
        quote! {
            #crate_path::plexus::PlexusError::InvalidParams {
                param: Some(#name.to_string()),
                problem: #crate_path::plexus::ParamProblem::Missing,
            }
        }
    };

    let extraction = match m.params.len() {
        0 => quote! {},
        1 => {
            let param = &m.params[0];
            let param_name = &param.name;
            let param_str = param_name.to_string();
            let is_option = crate::codegen::method_enum::is_option_type(&param.ty);
            let malformed = malformed(&param_str);
            let missing = missing(&param_str);
            // AUTH-13: a bare value (not an object) is this one param — a
            // shortcut, except for a method with a `target`: the gate reads
            // the target by name, and a bare value names none.
            let bare = if m.target.is_some() {
                quote! {
                    return Err(#crate_path::plexus::PlexusError::InvalidParams {
                        param: None,
                        problem: #crate_path::plexus::ParamProblem::Malformed {
                            detail: format!("params must be an object naming `{}`: this method's target is read by name", #param_str),
                        },
                    })
                }
            } else {
                quote! {
                    #deps::serde_json::from_value(params.clone())
                        .map_err(#malformed)?
                }
            };

            if is_option {
                // For Option<T>, missing field OR explicit null = None (not an error)
                quote! {
                    let #param_name = match &params {
                        #deps::serde_json::Value::Object(map) => {
                            map.get(#param_str)
                                .filter(|v| !v.is_null())  // Treat explicit null as missing
                                .map(|v| #deps::serde_json::from_value(v.clone()))
                                .transpose()
                                .map_err(#malformed)?
                        }
                        #deps::serde_json::Value::Null => None,
                        _ => #bare,
                    };
                }
            } else {
                // For required param, try to extract from object or deserialize directly
                quote! {
                    let #param_name = match &params {
                        #deps::serde_json::Value::Object(map) => {
                            if let Some(val) = map.get(#param_str) {
                                #deps::serde_json::from_value(val.clone())
                                    .map_err(#malformed)?
                            } else {
                                return Err(#missing);
                            }
                        }
                        _ => #bare,
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
                    let malformed = malformed(&name_str);
                    let missing = missing(&name_str);

                    if is_option {
                        // For Option<T>, missing field OR explicit null = None (not an error)
                        quote! {
                            let #name = map.get(#name_str)
                                .filter(|v| !v.is_null())  // Treat explicit null as missing
                                .map(|v| #deps::serde_json::from_value(v.clone()))
                                .transpose()
                                .map_err(#malformed)?;
                        }
                    } else {
                        // For required fields, error on missing
                        quote! {
                            let #name = map.get(#name_str)
                                .ok_or_else(|| #missing)
                                .and_then(|v| #deps::serde_json::from_value(v.clone())
                                    .map_err(#malformed))?;
                        }
                    }
                })
                .collect();

            quote! {
                let map = params.as_object()
                    .ok_or_else(|| #crate_path::plexus::PlexusError::InvalidParams {
                        param: None,
                        problem: #crate_path::plexus::ParamProblem::Malformed {
                            detail: "expected object".to_string(),
                        },
                    })?;
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

/// MACRO-14: bind `name` to what the resolver `expr` answers for
/// `&auth_ctx`, its error converted, not stringified: a `PlexusError` (a
/// typed `Forbidden` / `Unauthenticated`) passes through unchanged, any other
/// `Display` error is `ExecutionError(e.to_string())`. RETURNS-47: the route a
/// trait-impl method's verifier takes too.
pub(crate) fn resolve_typed(name: &syn::Ident, expr: &syn::Expr, crate_path: &syn::Path) -> TokenStream {
    quote! {
        let #name = match #expr(&auth_ctx).await {
            ::std::result::Result::Ok(v) => v,
            ::std::result::Result::Err(e) => {
                #[allow(dead_code)]
                struct __PlexusResolverErr<E>(::std::cell::Cell<::std::option::Option<E>>);
                #[allow(dead_code)]
                trait __PlexusResolverTyped {
                    fn __plexus_error(&self) -> #crate_path::plexus::PlexusError;
                }
                impl __PlexusResolverTyped for __PlexusResolverErr<#crate_path::plexus::PlexusError> {
                    fn __plexus_error(&self) -> #crate_path::plexus::PlexusError {
                        self.0.take().expect("resolver error taken once")
                    }
                }
                #[allow(dead_code)]
                trait __PlexusResolverDisplay {
                    fn __plexus_error(&self) -> #crate_path::plexus::PlexusError;
                }
                impl<E: ::std::fmt::Display> __PlexusResolverDisplay for &__PlexusResolverErr<E> {
                    fn __plexus_error(&self) -> #crate_path::plexus::PlexusError {
                        #crate_path::plexus::PlexusError::ExecutionError(
                            self.0.take().expect("resolver error taken once").to_string(),
                        )
                    }
                }
                let __err = __PlexusResolverErr(::std::cell::Cell::new(::std::option::Option::Some(e)));
                return ::std::result::Result::Err((&__err).__plexus_error());
            }
        };
    }
}
