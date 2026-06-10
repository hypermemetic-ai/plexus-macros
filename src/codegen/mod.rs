//! Code generation for Plexus RPC macros

mod activation;
mod method_enum;

use crate::parse::{
    extract_doc_description, find_and_validate_list_search_method, has_child_attr,
    has_method_attr, parse_deprecation_attrs, ChildMethodInfo, ChildMethodKind, HubMethodAttrs,
    HubMethodsAttrs, ListSearchKind, ListSearchMethodInfo, MethodInfo, ParsedDeprecation,
};
use proc_macro2::{Span, TokenStream};
use proc_macro_crate::{crate_name, FoundCrate};
use quote::{format_ident, quote};
use syn::{spanned::Spanned, FnArg, ImplItem, ItemImpl, Meta, Type};

/// Z2H-2: locate the `plexus-rpc` umbrella crate in the consumer's manifest.
///
/// Returns `Some(ident)` (rename-respecting) when the consuming crate depends
/// on `plexus-rpc`; `None` otherwise (including `FoundCrate::Itself`, which
/// only occurs for plexus-rpc's own examples/tests — those carry direct
/// dev-deps and take the direct-emission path).
pub(crate) fn umbrella_crate(err_span: Span) -> Option<syn::Ident> {
    match crate_name("plexus-rpc") {
        Ok(FoundCrate::Name(name)) => Some(syn::Ident::new(&name, err_span)),
        _ => None,
    }
}

/// Z2H-2: emit the per-expansion dependency shim module.
///
/// Every third-party name the activation macro emits (`async_trait`,
/// `futures`, `jsonrpsee`, `schemars`, `serde`, `serde_json`, `tokio`,
/// `uuid`) is referenced relatively through this module, so the umbrella /
/// direct decision is made ONCE here instead of at ~80 quote! sites:
///
/// - umbrella consumer (depends on `plexus-rpc`) → re-export from
///   `::plexus_rpc::__deps::*`; the consumer needs none of these crates in
///   its own Cargo.toml.
/// - direct consumer (today's sibling-checkout shape) → re-export from the
///   extern prelude, preserving pre-Z2H-2 semantics exactly.
///
/// The module ident is per-struct (`__plexus_deps_<Struct>`) so two
/// activations in one module don't collide.
pub(crate) fn deps_shim(shim_ident: &syn::Ident, err_span: Span) -> TokenStream {
    match umbrella_crate(err_span) {
        Some(umbrella) => quote! {
            #[doc(hidden)]
            #[allow(unused_imports, non_snake_case)]
            mod #shim_ident {
                pub use ::#umbrella::__deps::{
                    async_trait, futures, jsonrpsee, schemars, serde, serde_json, tokio, uuid,
                };
            }
        },
        None => quote! {
            #[doc(hidden)]
            #[allow(unused_imports, non_snake_case)]
            mod #shim_ident {
                pub use ::async_trait;
                pub use ::futures;
                pub use ::jsonrpsee;
                pub use ::schemars;
                pub use ::serde;
                pub use ::serde_json;
                pub use ::tokio;
                pub use ::uuid;
            }
        },
    }
}

/// Z2H-2: absolute path to `plexus-core` for derive-macro emission
/// (`#[derive(PlexusRequest)]`), which historically hardcoded
/// `::plexus_core::...`. Resolution is additive over that behavior: the only
/// case that changes is "no plexus-core dep but the plexus-rpc umbrella is
/// present", which now routes through `::<plexus_rpc>::core`.
pub(crate) fn derive_core_path(err_span: Span) -> syn::Path {
    match crate_name("plexus-core") {
        Ok(FoundCrate::Name(name)) => {
            let ident = syn::Ident::new(&name, err_span);
            syn::parse_quote!(::#ident)
        }
        // `Itself` only fires for plexus-core's own build/tests; integration
        // tests see the lib in the extern prelude, so the historical
        // hardcoded path keeps working there. Preserve it.
        Ok(FoundCrate::Itself) => syn::parse_quote!(::plexus_core),
        Err(_) => match umbrella_crate(err_span) {
            Some(umbrella) => syn::parse_quote!(::#umbrella::core),
            None => syn::parse_quote!(::plexus_core),
        },
    }
}

/// Z2H-2: absolute path to `serde_json` for derive-macro emission. Same
/// additive contract as [`derive_core_path`]: umbrella-only consumers route
/// through `::<plexus_rpc>::__deps::serde_json`; everyone else keeps the
/// historical `::serde_json`.
pub(crate) fn derive_serde_json_path(err_span: Span) -> syn::Path {
    match crate_name("serde_json") {
        Ok(FoundCrate::Name(name)) => {
            let ident = syn::Ident::new(&name, err_span);
            syn::parse_quote!(::#ident)
        }
        Ok(FoundCrate::Itself) => syn::parse_quote!(::serde_json),
        Err(_) => match umbrella_crate(err_span) {
            Some(umbrella) => syn::parse_quote!(::#umbrella::__deps::serde_json),
            None => syn::parse_quote!(::serde_json),
        },
    }
}

/// Resolve the `crate_path` to a syn::Path.
///
/// If the caller passed `crate_path = "..."` explicitly, parse that string.
/// Otherwise, call `proc_macro_crate::crate_name("plexus-core")` and emit the
/// correct path:
///   - `FoundCrate::Itself`    → `crate`  (code being compiled IS plexus-core)
///   - `FoundCrate::Name(n)`   → `::<n>`  (external; respects renaming)
///   - `Err` + umbrella        → `::<plexus_rpc>::core` (Z2H-2: the consumer
///                               depends only on the `plexus-rpc` umbrella)
///   - `Err`                   → a compile error naming the missing dependency
fn resolve_crate_path(explicit: Option<&str>, err_span: Span) -> syn::Result<syn::Path> {
    if let Some(s) = explicit {
        return syn::parse_str(s);
    }

    match crate_name("plexus-core") {
        Ok(FoundCrate::Itself) => syn::parse_str("crate"),
        Ok(FoundCrate::Name(name)) => {
            let ident = syn::Ident::new(&name, err_span);
            Ok(syn::parse_quote!(::#ident))
        }
        // Z2H-2: umbrella fallback — a consumer depending only on plexus-rpc
        // reaches plexus-core through the umbrella's re-export.
        Err(e) => match umbrella_crate(err_span) {
            Some(umbrella) => Ok(syn::parse_quote!(::#umbrella::core)),
            None => Err(syn::Error::new(
                err_span,
                format!(
                    "#[plexus_macros::activation] could not locate the `plexus-core` \
                     dependency in Cargo.toml ({e}). Add `plexus-core` (or the \
                     `plexus-rpc` umbrella) as a dependency, or pass \
                     `crate_path = \"...\"` explicitly to override."
                ),
            )),
        },
    }
}

/// Resolve the path to `plexus-auth-core` for AUTHZ-CRED-MACRO-3's
/// credentials-registry call site. Unlike `resolve_crate_path`, absence is
/// **not** an error — consuming crates that don't depend on
/// `plexus-auth-core` (e.g., plexus-substrate today) simply skip the
/// credentials wiring, leaving `MethodSchema.credentials` at its empty
/// default. This preserves behavior for all pre-AUTHZ-CRED-MACRO-3 callers
/// and lets new callers opt in by adding the dep.
///
/// Returns `Some(path)` when `plexus-auth-core` is reachable; `None`
/// otherwise. The path respects user renaming (e.g.,
/// `plexus_auth_core = { package = "plexus-auth-core" }`).
pub(crate) fn resolve_auth_core_path(err_span: Span) -> Option<syn::Path> {
    match crate_name("plexus-auth-core") {
        Ok(FoundCrate::Itself) => syn::parse_str("crate").ok(),
        Ok(FoundCrate::Name(name)) => {
            let ident = syn::Ident::new(&name, err_span);
            Some(syn::parse_quote!(::#ident))
        }
        // Z2H-2: umbrella fallback — plexus-rpc always bundles auth-core, so
        // an umbrella-only consumer gets the credentials wiring through the
        // re-export instead of silently skipping it.
        Err(_) => umbrella_crate(err_span)
            .map(|umbrella| syn::parse_quote!(::#umbrella::auth_core)),
    }
}

/// Generate all code from a #[hub_methods] impl block
pub fn generate_all(args: HubMethodsAttrs, mut input_impl: ItemImpl) -> syn::Result<TokenStream> {
    let crate_path: syn::Path =
        resolve_crate_path(args.crate_path.as_deref(), Span::call_site())?;

    // Get struct name (ident only, for enum naming)
    let struct_name = match &*input_impl.self_ty {
        Type::Path(type_path) => type_path
            .path
            .segments
            .last()
            .map(|s| s.ident.clone())
            .ok_or_else(|| syn::Error::new_spanned(&input_impl.self_ty, "Expected struct name"))?,
        _ => return Err(syn::Error::new_spanned(&input_impl.self_ty, "Expected struct type")),
    };

    // Get full self type (with generics) and impl generics for trait impl
    let self_ty = &input_impl.self_ty;
    let (impl_generics, _, where_clause) = input_impl.generics.split_for_impl();

    // Extract all #[method] / #[hub_method] functions and all #[child] functions.
    let mut methods: Vec<MethodInfo> = Vec::new();
    let mut child_methods: Vec<ChildMethodInfo> = Vec::new();

    for item in &mut input_impl.items {
        if let ImplItem::Fn(method) = item {
            // CHILD-3: a method annotated with both `#[child]` and `#[method]`
            // is a user error — the two roles are mutually exclusive.
            if has_child_attr(method) && has_method_attr(method) {
                return Err(syn::Error::new_spanned(
                    &method.sig,
                    "#[plexus_macros::child] and #[plexus_macros::method] are mutually exclusive \
                     on the same function: a function is either an RPC method or a child router \
                     entry, not both",
                ));
            }

            // CHILD-3: collect + validate `#[child]` methods. These are NOT
            // exposed as RPC methods; they contribute only to ChildRouter
            // routing. Strip the attribute after parsing so the emitted impl
            // block doesn't still carry the (now-no-op) `#[child]` macro
            // invocation.
            if has_child_attr(method) {
                child_methods.push(ChildMethodInfo::from_fn(method)?);
                // Strip #[child] and the companion #[plexus_macros::removed_in]
                // — they're both macro-only markers that rustc does not
                // otherwise recognize on free-standing methods.
                method.attrs.retain(|a| {
                    let last = a.path().segments.last().map(|s| s.ident.to_string());
                    !matches!(last.as_deref(), Some("child") | Some("removed_in"))
                });
                continue;
            }

            // Accept both the canonical #[method] / #[plexus_macros::method] and
            // the deprecated #[hub_method] / #[plexus_macros::hub_method] spellings.
            let hub_method_idx = method.attrs.iter().position(|attr| {
                let path = attr.path();
                let last = path.segments.last().map(|s| s.ident.to_string());
                matches!(last.as_deref(), Some("method") | Some("hub_method"))
            });

            if let Some(idx) = hub_method_idx {
                // Parse the attribute contents before removing
                let attr = method.attrs.remove(idx);
                let hub_method_attrs = match &attr.meta {
                    Meta::Path(_) => None, // #[hub_method] with no args
                    Meta::List(list) => {
                        Some(syn::parse2::<HubMethodAttrs>(list.tokens.clone())?)
                    }
                    Meta::NameValue(_) => None,
                };
                methods.push(MethodInfo::from_fn(method, hub_method_attrs.as_ref())?);

                // Strip #[plexus_macros::removed_in(...)] companion — rustc
                // doesn't know what to do with it, and the macro has already
                // folded its value into the `MethodInfo.deprecation` struct.
                method.attrs.retain(|a| {
                    let last = a.path().segments.last().map(|s| s.ident.to_string());
                    !matches!(last.as_deref(), Some("removed_in"))
                });

                // Strip #[from_auth(...)], #[activation_param], IR-5's
                // #[deprecated] / #[plexus_macros::removed_in], AND
                // AUTHZ-MACRO-PARAM-DOC-1's `///` doc comments from
                // parameters so rustc doesn't complain about unknown /
                // unstable attributes on parameters when the emitted
                // function signature is re-rendered. The doc-comment
                // text has already been captured into
                // `ParamInfo.description` by `MethodInfo::from_fn`.
                for arg in &mut method.sig.inputs {
                    if let FnArg::Typed(pat_type) = arg {
                        pat_type.attrs.retain(|attr| {
                            let last =
                                attr.path().segments.last().map(|s| s.ident.to_string());
                            !attr.path().is_ident("from_auth")
                                && !attr.path().is_ident("activation_param")
                                && !attr.path().is_ident("doc")
                                && !matches!(
                                    last.as_deref(),
                                    Some("deprecated") | Some("removed_in")
                                )
                        });
                    }
                }
            }
        }
    }

    // RED-3: reject `#[from_auth(...)]` on any method when the activation
    // has no `request = SomeType` declaration. Auth extraction requires
    // AuthContext to be populated by the activation-level PlexusRequest
    // extraction chain. Without `request = ...`, the resolver will
    // always receive `None` and every call 401s — ergonomics-bad, not
    // security-bad, but worth catching at compile time.
    if args.request_type.is_none() {
        if let Some(m) = methods.iter().find(|m| !m.auth_resolvers.is_empty()) {
            return Err(syn::Error::new(
                m.fn_name.span(),
                format!(
                    "RED-3: method `{}` uses `#[from_auth(...)]` but the activation \
                     declares no `request = SomeType`. Auth extraction requires a \
                     PlexusRequest struct to wire AuthContext through. Add \
                     `request = MyRequest` to `#[plexus::activation(...)]`, or remove \
                     the `#[from_auth]` annotation.",
                    m.method_name
                ),
            ));
        }
    }

    // REQ-10: validate per-method request overrides against the activation's
    // `required = [...]` lock list. If the activation has any required fields
    // and a method uses `request = ()`, emit a compile error naming the
    // fields that would be dropped.
    if !args.required_fields.is_empty() {
        for m in &methods {
            if matches!(m.request_override, Some(crate::parse::MethodRequestOverride::Skip)) {
                let required_names: Vec<String> = args.required_fields
                    .iter()
                    .map(|i| i.to_string())
                    .collect();
                return Err(syn::Error::new(
                    m.fn_name.span(),
                    format!(
                        "REQ-10: method `{}` uses `request = ()` but activation's `required = [{}]` \
                         locks these fields. Either remove the method-level `request = ()` override \
                         or remove the activation's `required` list (or move the method to a \
                         different activation).",
                        m.method_name,
                        required_names.join(", ")
                    ),
                ));
            }
        }
    }

    // IR-5: capture activation-level `#[deprecated]` (+ optional
    // `#[plexus_macros::removed_in]`) from the impl block's attributes
    // BEFORE the IR-3 cleanup below strips the `#[removed_in]` companion.
    // Threads through to `activation::generate` which emits
    // `PluginSchema.deprecation` when populated.
    let activation_deprecation: Option<ParsedDeprecation> =
        parse_deprecation_attrs(&input_impl.attrs, input_impl.span())?;

    // IR-3/IR-5: strip any residual `#[plexus_macros::removed_in(...)]`
    // attribute that landed on a bare (non-#[method], non-#[child]) impl
    // item, or on the impl block itself — the companion attribute is
    // macro-only and rustc doesn't know how to handle it.
    //
    // Also strip the IR-5 `#[doc = "__plexus_removed_in:X"]` sentinel that
    // the outer-expansion path uses to bridge impl-block-level `#[removed_in]`
    // past its own premature expansion — the activation macro has already
    // folded it into `activation_deprecation` by now.
    input_impl.attrs.retain(|a| {
        let last = a.path().segments.last().map(|s| s.ident.to_string());
        if matches!(last.as_deref(), Some("removed_in")) {
            return false;
        }
        if a.path().is_ident("doc") {
            if let syn::Meta::NameValue(syn::MetaNameValue { value, .. }) = &a.meta {
                if let syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(s),
                    ..
                }) = value
                {
                    if s.value().starts_with("__plexus_removed_in:") {
                        return false;
                    }
                }
            }
        }
        true
    });
    for item in &mut input_impl.items {
        if let ImplItem::Fn(f) = item {
            f.attrs.retain(|a| {
                let last = a.path().segments.last().map(|s| s.ident.to_string());
                !matches!(last.as_deref(), Some("removed_in"))
            });
        }
    }

    // CHILD-3: at most one dynamic `#[child]` method per impl.
    let dynamic_children: Vec<&ChildMethodInfo> = child_methods
        .iter()
        .filter(|c| matches!(c.kind, ChildMethodKind::Dynamic))
        .collect();
    if dynamic_children.len() > 1 {
        let names: Vec<String> = dynamic_children
            .iter()
            .map(|c| c.fn_name.to_string())
            .collect();
        return Err(syn::Error::new_spanned(
            &input_impl,
            format!(
                "at most one dynamic #[child] method is allowed per activation impl; \
                 found {}: {}",
                dynamic_children.len(),
                names.join(", ")
            ),
        ));
    }

    // CHILD-3: reject mixing the legacy `children = [...]` attribute with
    // the new `#[child]` method attribute on the same impl — the two
    // systems don't merge, and silently picking one would hide intent.
    if !child_methods.is_empty() && !args.children.is_empty() {
        return Err(syn::Error::new_spanned(
            &input_impl,
            "#[plexus_macros::child] methods cannot be combined with the legacy \
             `children = [...]` activation attribute on the same impl; choose one. \
             Remove `children = [...]` to migrate to the method-based syntax.",
        ));
    }

    // CHILD-4: resolve `list = "..."` / `search = "..."` args against the
    // same-impl sibling methods. Missing names produce `not found in impl`;
    // signature mismatches produce an error containing `signature mismatch`.
    let mut list_method: Option<ListSearchMethodInfo> = None;
    let mut search_method: Option<ListSearchMethodInfo> = None;
    for child in &child_methods {
        if let Some(name) = &child.list_fn {
            let info = find_and_validate_list_search_method(
                &input_impl.items,
                name,
                ListSearchKind::List,
                &child.fn_name,
            )?;
            if list_method.is_some() {
                return Err(syn::Error::new(
                    name.span(),
                    format!(
                        "duplicate `list = \"{}\"` across #[plexus_macros::child] methods; \
                         at most one list method per activation impl",
                        name
                    ),
                ));
            }
            list_method = Some(info);
        }
        if let Some(name) = &child.search_fn {
            let info = find_and_validate_list_search_method(
                &input_impl.items,
                name,
                ListSearchKind::Search,
                &child.fn_name,
            )?;
            if search_method.is_some() {
                return Err(syn::Error::new(
                    name.span(),
                    format!(
                        "duplicate `search = \"{}\"` across #[plexus_macros::child] methods; \
                         at most one search method per activation impl",
                        name
                    ),
                ));
            }
            search_method = Some(info);
        }
    }

    if methods.is_empty() {
        return Err(syn::Error::new_spanned(
            &input_impl,
            "No #[plexus::method] functions found",
        ));
    }

    // CHILD-8: Mixing the explicit `hub` flag with `#[child]` methods is
    // ambiguous — both are mechanisms for declaring "this activation is a
    // hub". Require the user to pick exactly one.
    if args.hub && !child_methods.is_empty() {
        return Err(syn::Error::new_spanned(
            &input_impl,
            "#[plexus_macros::activation(hub, ...)] cannot be combined with #[plexus_macros::child] \
             methods — the macro infers hub-mode from the presence of #[child] methods. Remove the \
             'hub' flag or remove the #[child] methods.",
        ));
    }

    // CHILD-8: hub-mode is inferred from either the explicit `hub` flag OR
    // the presence of any `#[child]` method. Both sources feed into
    // `call_fallback` / `plugin_schema_body` codegen. The explicit flag is
    // also threaded through separately so `child_router_impl` can still
    // recognize the legacy Solar pattern (hand-written router + no `#[child]`).
    let effective_hub = args.hub || !child_methods.is_empty();
    let hub_explicit = args.hub;

    // CHILD-8: detect whether the impl already carries a `fn plugin_children(&self)`
    // so the macro doesn't shadow a user-written one when synthesizing. We scan
    // for an ImplItem::Fn named `plugin_children` whose first parameter is a
    // receiver (`&self`). Anything else (same name, different shape) is treated
    // as "not defined" for synthesis purposes — the user's item compiles as-is
    // and any collision surfaces as a regular Rust name-clash error.
    let impl_defines_plugin_children = input_impl.items.iter().any(|item| {
        if let ImplItem::Fn(f) = item {
            if f.sig.ident == "plugin_children" {
                return matches!(f.sig.inputs.first(), Some(FnArg::Receiver(_)));
            }
        }
        false
    });

    // CHILD-8: synthesize `plugin_children` when at least one `#[child]` method
    // is present and the impl doesn't already define one. Dynamic `#[child]`
    // methods are intentionally omitted — a finite `ChildSummary` list isn't
    // meaningful for an open-ended name set; dynamic children are discoverable
    // via `ChildRouter::list_children` at runtime when opted in.
    let synthesize_plugin_children =
        !child_methods.is_empty() && !impl_defines_plugin_children;

    // Resolve the activation description with this precedence:
    //   1. Explicit `description = "..."` on #[plexus_macros::activation(...)] wins.
    //   2. Otherwise fall back to `///` doc comments on the impl block, joined with
    //      '\n' and with common leading whitespace stripped (same rule as `cargo doc`).
    //   3. If neither is present, description is the empty string.
    let resolved_description: String = args
        .description
        .clone()
        .or_else(|| extract_doc_description(&input_impl.attrs))
        .unwrap_or_default();

    // IR-10: every `#[child]` method emits a `MethodSchema` entry unconditionally,
    // regardless of whether `plugin_children()` is hand-written or synthesized
    // (CHILD-8 inferred hub). IR-4 relaxed plexus-core's `validate_no_collisions`
    // to recognize that a static-child method and the derived `ChildSummary`
    // of the same name are the same thing by construction, not a collision.
    // Downstream consumers (synapse, hub-codegen, any IR reader) see the full
    // role-tagged method list.
    let schema_child_methods: Vec<ChildMethodInfo> = child_methods.clone();

    // Strip the `#[deprecated]` attribute from the impl block itself so the
    // emitted trait impl doesn't carry it (rustc accepts it but reports
    // false-positive "use of deprecated ..." warnings inside the generated
    // code). `#[plexus_macros::removed_in]` is already stripped earlier.
    input_impl.attrs.retain(|a| !a.path().is_ident("deprecated"));

    // IR-5: deprecation warnings for attribute args `hub` and `children = [...]`.
    let hub_deprecation_warning =
        hub_flag_deprecation_warning(args.hub, args.hub_span, &struct_name);
    let children_deprecation_warning = children_arg_deprecation_warning(
        !args.children.is_empty(),
        args.children_span,
        &struct_name,
    );

    // RED-6: enforce declared auth posture.
    //
    // - `auth = "required"`: every method must either use #[from_auth] OR
    //   carry #[plexus_macros::method(public)]. Compile error otherwise.
    // - `auth = "none"`: no method may use #[from_auth]. Compile error if any.
    // - `auth = "optional"` (default): no enforcement. RED-2 emits a warning
    //   when auth is asymmetric across methods in this activation.
    // - `auth = "mixed"`: like optional but suppresses the RED-2 warning
    //   (explicit acknowledgment of the asymmetry).
    //
    // AUTHZ-MACRO-OPTIONAL-AUTH-1 carve-out: a method whose signature is
    // `auth: Option<&AuthContext>` is an explicit opt-in to anonymous-friendly
    // handling. Under `auth = "required"` we downgrade the previous hard error
    // to a compile-time warning ("handler accepts anonymous but activation
    // requires auth") so the trak-style "anonymous read, authed write"
    // pattern is reachable.
    // R-1 (joint AP-1): enforcement treats an undeclared posture as
    // `Optional` (unchanged RED-6 semantics); only a DECLARED posture is
    // emitted into the generated MethodSchema entries (see
    // `method_enum::generate`).
    let effective_auth_posture = args.auth_posture.unwrap_or_default();
    let mut required_posture_optional_auth_methods: Vec<&crate::parse::MethodInfo> = Vec::new();
    match effective_auth_posture {
        crate::parse::AuthPosture::Required => {
            for m in &methods {
                if m.auth_resolvers.is_empty() && !m.public {
                    if m.auth_optional {
                        required_posture_optional_auth_methods.push(m);
                        continue;
                    }
                    return Err(syn::Error::new(
                        m.fn_name.span(),
                        format!(
                            "RED-6: activation `{}` is `auth = \"required\"` but method \
                             `{}` has no #[from_auth(...)] and no \
                             #[plexus_macros::method(public)] acknowledgment. \
                             Either gate the method with #[from_auth(resolver)] \
                             or explicitly mark it public with \
                             #[plexus_macros::method(public)].",
                            struct_name, m.method_name,
                        ),
                    ));
                }
            }
        }
        crate::parse::AuthPosture::None => {
            for m in &methods {
                if !m.auth_resolvers.is_empty() {
                    return Err(syn::Error::new(
                        m.fn_name.span(),
                        format!(
                            "RED-6: activation `{}` is `auth = \"none\"` (affirmative public) \
                             but method `{}` uses #[from_auth(...)]. Remove the \
                             annotation or change the activation's auth posture.",
                            struct_name, m.method_name,
                        ),
                    ));
                }
            }
        }
        crate::parse::AuthPosture::Mixed | crate::parse::AuthPosture::Optional => {}
    }
    let required_posture_optional_auth_warning = required_posture_optional_auth_warning(
        &required_posture_optional_auth_methods,
        &struct_name,
    );

    // RED-2: warn when the activation has methods with #[from_auth] and other
    // methods without. Suppressed under `auth = "mixed"` (explicit) and the
    // two strict postures (where compile-time errors already cover the cases).
    let suppress_mixed_warning =
        !matches!(effective_auth_posture, crate::parse::AuthPosture::Optional);
    let mixed_auth_warning = mixed_auth_warning(&methods, suppress_mixed_warning, &struct_name);

    // AUTHZ-CRED-MACRO-3: resolve the optional `plexus-auth-core` path used by
    // the credentials-registry call site in `compute_method_schemas`. None
    // means the consuming crate doesn't depend on `plexus-auth-core` and so
    // the macro should emit empty credential metadata (preserving pre-ticket
    // behavior).
    let auth_core_path = resolve_auth_core_path(Span::call_site());

    // Z2H-2: per-expansion dependency shim. Every third-party name in the
    // emitted code resolves relatively through this module; whether it
    // re-exports `::plexus_rpc::__deps::*` (umbrella consumer) or the extern
    // prelude (direct consumer) is decided once in `deps_shim`.
    let deps = format_ident!("__plexus_deps_{}", struct_name);
    let deps_shim_mod = deps_shim(&deps, Span::call_site());

    // Generate pieces
    let method_enum = method_enum::generate(
        &struct_name,
        &methods,
        &schema_child_methods,
        &crate_path,
        auth_core_path.as_ref(),
        args.request_type.as_ref(),  // REQ-6: activation-level request type (for runtime merge)
        &deps,
        args.auth_posture, // R-1: declared posture (None = silent, pre-R-1 wire)
    );
    let activation_impl = activation::generate(
        &struct_name,
        self_ty,
        &impl_generics,
        where_clause,
        &args.namespace,
        args.version.as_deref(),
        &resolved_description,
        args.long_description.as_deref(),
        &methods,
        &crate_path,
        args.resolve_handle,
        effective_hub,
        hub_explicit,
        synthesize_plugin_children,
        args.plugin_id.as_deref(),
        args.namespace_fn.as_deref(),
        args.request_type.as_ref(),
        &args.children,
        &child_methods,
        list_method.as_ref(),
        search_method.as_ref(),
        activation_deprecation.as_ref(),
        &deps,
    );

    Ok(quote! {
        #deps_shim_mod
        #input_impl
        #method_enum
        #activation_impl
        #hub_deprecation_warning
        #children_deprecation_warning
        #mixed_auth_warning
        #required_posture_optional_auth_warning
    })
}

/// AUTHZ-MACRO-OPTIONAL-AUTH-1: emit a compile-time warning when an
/// activation declares `auth = "required"` and one of its methods uses the
/// optional-auth handler shape `auth: Option<&AuthContext>`.
///
/// The combination is legal — the optional shape was added precisely so an
/// activation can serve anonymous reads alongside authed writes — but it's
/// worth flagging because the activation's declared posture says "every
/// caller must authenticate." Uses the same dummy-deprecated-const trick as
/// the IR-5 / RED-2 warnings.
fn required_posture_optional_auth_warning(
    methods: &[&crate::parse::MethodInfo],
    struct_name: &syn::Ident,
) -> TokenStream {
    if methods.is_empty() {
        return TokenStream::new();
    }
    let names = methods
        .iter()
        .map(|m| m.method_name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let note = format!(
        "plexus-macros (AUTHZ-MACRO-OPTIONAL-AUTH-1): activation `{}` declares \
         `auth = \"required\"` but the following method(s) accept anonymous \
         callers via `auth: Option<&AuthContext>`: {}. The handlers will see \
         `None` when no auth context is attached. If this is intentional, \
         consider `auth = \"mixed\"` to silence this warning; otherwise \
         change the handler signature to `auth: &AuthContext` (or use \
         `#[from_auth(...)]`).",
        struct_name, names,
    );
    let span = methods
        .first()
        .map(|m| m.fn_name.span())
        .unwrap_or_else(|| struct_name.span());
    let const_name = format_ident!(
        "_PLEXUS_MACROS_REQUIRED_POSTURE_OPTIONAL_AUTH_WARNING_{}",
        struct_name,
        span = span,
    );
    let touch_fn_name = format_ident!(
        "_plexus_macros_touch_required_posture_optional_auth_warning_{}",
        struct_name,
        span = span,
    );
    quote::quote_spanned! { span =>
        #[doc(hidden)]
        #[allow(non_upper_case_globals, non_snake_case, dead_code)]
        #[deprecated(since = "0.5", note = #note)]
        const #const_name: () = ();

        #[doc(hidden)]
        #[allow(non_snake_case, dead_code)]
        fn #touch_fn_name() {
            let _ = #const_name;
        }
    }
}

/// IR-5: emit a compile-time deprecation warning when
/// `#[plexus_macros::activation(hub)]` is used. Generates a `#[deprecated]`
/// dummy const and a touch-site that fires rustc's `deprecated` lint
/// at the span of the flag. Stable-compatible alternative to
/// `proc_macro::Diagnostic`.
fn hub_flag_deprecation_warning(
    hub_is_set: bool,
    hub_span: Option<Span>,
    struct_name: &syn::Ident,
) -> TokenStream {
    if !hub_is_set {
        return TokenStream::new();
    }
    let span = hub_span.unwrap_or_else(|| struct_name.span());
    let const_name = format_ident!(
        "_PLEXUS_MACROS_DEPRECATED_HUB_FLAG_{}",
        struct_name,
        span = span
    );
    // The trick: declare the deprecated const at module scope, and touch it
    // from a same-module hidden function. Rustc's `deprecated` lint fires at
    // the touch site because the function body is NOT inside the const's
    // own initializer (where the lint is suppressed as a consistency rule).
    let touch_fn_name = format_ident!(
        "_plexus_macros_touch_deprecated_hub_flag_{}",
        struct_name,
        span = span
    );
    quote::quote_spanned! { span =>
        #[doc(hidden)]
        #[allow(non_upper_case_globals, non_snake_case, dead_code)]
        #[deprecated(
            since = "0.5",
            note = "plexus-macros: the 'hub' argument is deprecated; hub mode is inferred automatically from #[child]-tagged methods. This argument will be removed in plexus-macros 0.6."
        )]
        const #const_name: () = ();

        #[doc(hidden)]
        #[allow(non_snake_case, dead_code)]
        fn #touch_fn_name() {
            // Referencing the deprecated const here fires rustc's
            // `deprecated` lint, pointing at the span of the `hub` flag.
            let _ = #const_name;
        }
    }
}

/// IR-5: emit a compile-time deprecation warning when
/// `#[plexus_macros::activation(children = [...])]` is used. Companion to
/// `hub_flag_deprecation_warning`; uses the same dummy-const mechanism.
fn children_arg_deprecation_warning(
    children_is_set: bool,
    children_span: Option<Span>,
    struct_name: &syn::Ident,
) -> TokenStream {
    if !children_is_set {
        return TokenStream::new();
    }
    let span = children_span.unwrap_or_else(|| struct_name.span());
    let const_name = format_ident!(
        "_PLEXUS_MACROS_DEPRECATED_CHILDREN_ARG_{}",
        struct_name,
        span = span
    );
    let touch_fn_name = format_ident!(
        "_plexus_macros_touch_deprecated_children_arg_{}",
        struct_name,
        span = span
    );
    quote::quote_spanned! { span =>
        #[doc(hidden)]
        #[allow(non_upper_case_globals, non_snake_case, dead_code)]
        #[deprecated(
            since = "0.5",
            note = "plexus-macros: the 'children' attribute argument is deprecated; use #[plexus_macros::child] on the accessor method(s) instead. This argument will be removed in plexus-macros 0.6."
        )]
        const #const_name: () = ();

        #[doc(hidden)]
        #[allow(non_snake_case, dead_code)]
        fn #touch_fn_name() {
            let _ = #const_name;
        }
    }
}

/// RED-2: emit a compile-time warning when an activation has a mix of
/// methods that use `#[from_auth(...)]` and methods that don't. The
/// asymmetry is almost always a bug — a developer added a new method
/// to an auth'd activation and forgot the annotation.
///
/// Suppression mechanisms:
/// - Activation declares `auth_posture = "mixed"` (intentional)
/// - Each non-auth method carries `#[plexus_macros::method(public)]`
///
/// Uses the same dummy-deprecated-const pattern as the IR-5 warnings.
fn mixed_auth_warning(
    methods: &[crate::parse::MethodInfo],
    auth_posture_mixed: bool,
    struct_name: &syn::Ident,
) -> TokenStream {
    if auth_posture_mixed {
        return TokenStream::new();
    }
    let any_auth = methods.iter().any(|m| !m.auth_resolvers.is_empty());
    if !any_auth {
        return TokenStream::new();
    }
    let unauthed: Vec<&crate::parse::MethodInfo> = methods
        .iter()
        .filter(|m| m.auth_resolvers.is_empty() && !m.public)
        .collect();
    if unauthed.is_empty() {
        return TokenStream::new();
    }
    let names = unauthed
        .iter()
        .map(|m| m.method_name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let note = format!(
        "plexus-macros (RED-2): activation `{}` mixes auth and non-auth \
         methods. The following methods do NOT have `#[from_auth(...)]`: {}. \
         If this is intentional, add `auth_posture = \"mixed\"` to the \
         activation attribute, or `#[plexus_macros::method(public)]` to each \
         intentionally-public method. Otherwise, the missing methods will \
         dispatch unauthenticated.",
        struct_name,
        names,
    );
    let span = unauthed
        .first()
        .map(|m| m.fn_name.span())
        .unwrap_or_else(|| struct_name.span());
    let const_name = format_ident!(
        "_PLEXUS_MACROS_MIXED_AUTH_WARNING_{}",
        struct_name,
        span = span,
    );
    let touch_fn_name = format_ident!(
        "_plexus_macros_touch_mixed_auth_warning_{}",
        struct_name,
        span = span,
    );
    quote::quote_spanned! { span =>
        #[doc(hidden)]
        #[allow(non_upper_case_globals, non_snake_case, dead_code)]
        #[deprecated(since = "0.5", note = #note)]
        const #const_name: () = ();

        #[doc(hidden)]
        #[allow(non_snake_case, dead_code)]
        fn #touch_fn_name() {
            let _ = #const_name;
        }
    }
}
