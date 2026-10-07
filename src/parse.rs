//! Attribute parsing for Plexus RPC macros

use std::collections::HashMap;
use syn::{
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
    spanned::Spanned,
    Expr, ExprLit, ExprPath, ExprTuple, FnArg, GenericArgument, ImplItem, ImplItemFn, Lit, Meta,
    MetaList, MetaNameValue, Pat, PathArguments, ReturnType, Token, Type,
};

/// A parameter resolved from the auth context via an arbitrary expression.
/// Created by `#[from_auth(self.db.validate_user)]` on a method parameter.
#[derive(Debug, Clone)]
pub struct AuthResolver {
    /// The parameter name in the method signature
    pub param_name: syn::Ident,
    /// The resolver expression (e.g. `self.db.validate_user`)
    pub resolver_expr: syn::Expr,
}

/// The fallback value written into `DeprecationInfo.removed_in` when a method
/// is tagged with `#[deprecated]` but neither supplies a `removed_in` key
/// (which rustc ignores) nor a companion `#[plexus_macros::removed_in("X")]`
/// attribute.
///
/// Matches the ticket IR-3 specification (Context table, row "None +
/// `#[deprecated(...)]`"): `removed_in: "unspecified"`.
pub(crate) const DEPRECATION_REMOVED_IN_UNSPECIFIED: &str = "unspecified";

/// Parsed `DeprecationInfo` assembled from a method's `#[deprecated(...)]` and
/// `#[plexus_macros::removed_in("X")]` attributes.
///
/// Fields mirror `plexus_core::plexus::DeprecationInfo` — the codegen layer
/// translates this into a `MethodSchema.deprecation = Some(DeprecationInfo { .. })`
/// initializer.
#[derive(Debug, Clone)]
pub struct ParsedDeprecation {
    /// Version at which deprecation began (`since` on `#[deprecated]`, or
    /// empty string if omitted).
    pub since: String,
    /// Planned removal version. Sourced from either `#[plexus_macros::removed_in("X")]`
    /// or a `removed_in = "X"` key inside `#[deprecated(...)]` (rustc ignores
    /// the latter; we don't). Falls back to
    /// `DEPRECATION_REMOVED_IN_UNSPECIFIED` when neither is supplied.
    pub removed_in: String,
    /// Migration guidance (`note` on `#[deprecated]`, or empty string).
    pub message: String,
}

/// Bidirectional channel type configuration
#[derive(Debug, Clone)]
pub enum BidirType {
    /// Not bidirectional
    None,
    /// Use StandardBidirChannel (StandardRequest, StandardResponse)
    Standard,
    /// Use custom request/response types
    Custom { request: String, response: String },
}

/// Per-method override for the activation-level request type.
///
/// Used in `#[plexus::method(request = ())]` to exempt individual methods from
/// the activation-level request extraction. When set to `Skip`, the dispatch
/// wrapper omits the `<ReqType>::extract(raw_ctx)?` call for that method, even
/// if the activation declares `request = MyType`.
///
/// `Type` variant is reserved for future use (per-method request type override).
#[derive(Debug, Clone)]
pub enum MethodRequestOverride {
    /// `request = ()` — skip extraction for this method entirely.
    Skip,
    /// `request = SomeType` — use a different request type for this method (future).
    #[allow(dead_code)]
    Type(syn::Type),
}

/// RETURNS-8: `#[method(streaming)]` is gone — a method's cardinality is
/// read from its return type (RETURNS-1 decision 2).
pub(crate) const STREAMING_REMOVED: &str =
    "`streaming` is removed: cardinality comes from the return type — remove `streaming`. \
     A method returning `-> T` or `-> Result<T, E>` yields one item; one returning \
     `-> impl Stream<Item = ...>` yields many.";

/// RETURNS-8: `#[method(returns(..))]` is gone — it filtered the variants of
/// a shared event enum, and a method's own return type now says exactly what
/// it yields (RETURNS-1 decision 16).
pub(crate) const RETURNS_REMOVED: &str =
    "`returns(..)` is removed: a method's return type is its contract. Instead of \
     filtering a shared event enum's variants, give each method its own return type — \
     e.g. `-> Result<Ticket, GetError>` or `-> impl Stream<Item = ListItem>`.";

/// Parsed attributes for #[hub_method]
pub struct HubMethodAttrs {
    pub name: Option<String>,
    /// Explicit description from `description = "..."` attribute argument.
    /// When `Some`, this wins over `///` doc comments. When `None`, the method's
    /// doc comments are used as the default description (see `MethodInfo::from_fn`).
    pub description: Option<String>,
    /// Parameter descriptions: param_name -> description
    pub param_docs: HashMap<String, String>,
    /// Bidirectional channel type (from #[hub_method(bidirectional)] or #[hub_method(bidirectional(request = "...", response = "..."))])
    pub bidirectional: BidirType,
    /// HTTP method for REST endpoints (GET, POST, PUT, DELETE, PATCH)
    /// Stored as string during parsing, converted to enum in code generation
    pub http_method: Option<String>,
    /// Per-method request override. `Some(Skip)` means `request = ()` — this method
    /// bypasses the activation-level request extraction even if the activation has
    /// `request = MyType`. `None` means inherit the activation-level behavior.
    pub request_override: Option<MethodRequestOverride>,
    /// RED-2: explicit acknowledgment that this method is intentionally
    /// public (no auth). Set by `#[plexus_macros::method(public)]`. Suppresses
    /// the mixed-auth warning for this method.
    ///
    /// R-1 (AUTHZ-CORE-2): also emitted into the generated `MethodSchema` as
    /// the `public` flag — the method is exempt from the default-deny gate.
    /// `public` and `scope = "..."` together is a compile error (a public
    /// method has no required scope to enforce).
    pub public: bool,
    /// R-1 (AUTHZ-CORE-2): explicit required scopes from one or more
    /// `scope = "..."` arguments. Multi-scope means conjunction — the caller
    /// must satisfy ALL listed scopes (AND, not OR; AUTHZ-S01-output §4).
    /// Empty means the implicit `<activation>.<method>` rule applies at
    /// registry-build time (AUTHZ-CORE-4's `ScopeRegistry` resolves it; the
    /// IR carries nothing for the implicit case).
    ///
    /// Each literal is validated against the AUTHZ-CORE-4 scope grammar at
    /// macro-expansion time (see [`validate_scope_literal`]).
    pub scopes: Vec<String>,
}

/// R-1 (AUTHZ-CORE-2 acceptance criterion #3): validate a `scope = "..."`
/// literal against the AUTHZ-CORE-4 scope grammar pinned by AUTHZ-S01-output
/// §1/§4:
///
/// - bare `*` ("all scopes"), or
/// - ASCII dot-separated segments, each matching `[a-z][a-z0-9_]*`,
/// - optionally ending in a `.*` segment wildcard (segment-bounded:
///   `cone.*` matches `cone.send_message`, not `forms.list`).
///
/// Returns a human-readable description of the violation on failure. The
/// canonical `Scope::try_new` (AUTHZ-CORE-4 / R-0) enforces the same grammar
/// at registry-build time; this compile-time check fails fast at the literal.
pub fn validate_scope_literal(s: &str) -> Result<(), String> {
    if s == "*" {
        return Ok(());
    }
    if s.is_empty() {
        return Err("scope must not be empty".to_string());
    }
    let segments: Vec<&str> = s.split('.').collect();
    for (i, seg) in segments.iter().enumerate() {
        let is_last = i == segments.len() - 1;
        if *seg == "*" {
            if is_last && segments.len() > 1 {
                continue; // trailing `.*` segment wildcard
            }
            return Err(
                "`*` is only valid as the entire scope or as the trailing `.*` segment"
                    .to_string(),
            );
        }
        let mut chars = seg.chars();
        let valid = match chars.next() {
            Some(c) if c.is_ascii_lowercase() => {
                chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
            }
            _ => false,
        };
        if !valid {
            return Err(format!(
                "segment `{}` must match [a-z][a-z0-9_]*",
                seg
            ));
        }
    }
    Ok(())
}

impl Parse for HubMethodAttrs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut name = None;
        let mut description: Option<String> = None;
        let mut param_docs = HashMap::new();
        let mut bidirectional = BidirType::None;
        let mut http_method = None;
        let mut request_override: Option<MethodRequestOverride> = None;
        let mut public = false;
        // R-1 (AUTHZ-CORE-2): explicit `scope = "..."` literals plus the
        // spans needed to anchor the `public` + `scope` contradiction error.
        let mut scopes: Vec<String> = Vec::new();
        let mut public_span: Option<proc_macro2::Span> = None;
        let mut first_scope_span: Option<proc_macro2::Span> = None;

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
                                 See plexus.call implementation for an example.",
                            ));
                        } else if path.is_ident("streaming") {
                            // RETURNS-8: cardinality is read from the signature.
                            return Err(syn::Error::new_spanned(&path, STREAMING_REMOVED));
                        } else if path.is_ident("bidirectional") {
                            bidirectional = BidirType::Standard;
                        } else if path.is_ident("public") {
                            // RED-2: explicit acknowledgment that this method
                            // intentionally bypasses activation-level auth.
                            // R-1: also emitted as the MethodSchema `public`
                            // flag (exempt from the default-deny gate).
                            public = true;
                            public_span = Some(path.span());
                        }
                    }
                    Meta::NameValue(MetaNameValue { path, value, .. }) => {
                        if path.is_ident("name") {
                            if let Expr::Lit(ExprLit { lit: Lit::Str(s), .. }) = value {
                                name = Some(s.value());
                            }
                        } else if path.is_ident("description") {
                            if let Expr::Lit(ExprLit { lit: Lit::Str(s), .. }) = value {
                                description = Some(s.value());
                            }
                        } else if path.is_ident("http_method") {
                            if let Expr::Lit(ExprLit { lit: Lit::Str(s), .. }) = value {
                                let method = s.value().to_uppercase();
                                // Validate HTTP method
                                match method.as_str() {
                                    "GET" | "POST" | "PUT" | "DELETE" | "PATCH" => {
                                        http_method = Some(method);
                                    }
                                    _ => {
                                        return Err(syn::Error::new_spanned(
                                            s,
                                            format!(
                                                "Invalid HTTP method '{}'. Valid methods: GET, POST, PUT, DELETE, PATCH",
                                                method
                                            ),
                                        ));
                                    }
                                }
                            }
                        } else if path.is_ident("scope") {
                            // R-1 (AUTHZ-CORE-2): explicit required scope.
                            // Repeatable — `scope = "x.y", scope = "x.z"`
                            // means the caller must satisfy ALL (AND).
                            let Expr::Lit(ExprLit { lit: Lit::Str(s), .. }) = value else {
                                return Err(syn::Error::new_spanned(
                                    &path,
                                    "scope = ... expects a string literal, e.g. \
                                     #[plexus_macros::method(scope = \"facet.write\")]",
                                ));
                            };
                            let literal = s.value();
                            if let Err(why) = validate_scope_literal(&literal) {
                                return Err(syn::Error::new(
                                    s.span(),
                                    format!(
                                        "invalid scope literal `{}`: {}. Scopes are ASCII \
                                         dot-separated segments matching [a-z][a-z0-9_]*, \
                                         optionally ending in a `.*` segment wildcard, or \
                                         the bare `*` (AUTHZ-CORE-4 scope grammar; see \
                                         `Scope::try_new`).",
                                        literal, why,
                                    ),
                                ));
                            }
                            if first_scope_span.is_none() {
                                first_scope_span = Some(s.span());
                            }
                            scopes.push(literal);
                        } else if path.is_ident("request") {
                            // request = ()  → skip activation-level request extraction
                            // request = SomeType → use a different request type (future)
                            match value {
                                Expr::Tuple(ExprTuple { ref elems, .. }) if elems.is_empty() => {
                                    request_override = Some(MethodRequestOverride::Skip);
                                }
                                Expr::Path(ExprPath { ref path, .. }) => {
                                    // Convert the path expression to a Type
                                    let ty: Type = Type::Path(syn::TypePath {
                                        qself: None,
                                        path: path.clone(),
                                    });
                                    request_override = Some(MethodRequestOverride::Type(ty));
                                }
                                _ => {
                                    // Unknown form — ignore for forward compatibility
                                }
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
                            // RETURNS-8: a method's return type is its
                            // contract; there is no shared enum to filter.
                            return Err(syn::Error::new_spanned(&path, RETURNS_REMOVED));
                        } else if path.is_ident("bidirectional") {
                            // Parse bidirectional(request = "MyReq", response = "MyResp")
                            let parser = Punctuated::<Meta, Token![,]>::parse_terminated;
                            let nested = syn::parse::Parser::parse2(parser, tokens.clone())?;

                            let mut request = None;
                            let mut response = None;

                            for meta in nested {
                                if let Meta::NameValue(MetaNameValue { path, value, .. }) = meta {
                                    if let Expr::Lit(ExprLit { lit: Lit::Str(s), .. }) = value {
                                        if path.is_ident("request") {
                                            request = Some(s.value());
                                        } else if path.is_ident("response") {
                                            response = Some(s.value());
                                        }
                                    }
                                }
                            }

                            match (request, response) {
                                (Some(req), Some(resp)) => {
                                    bidirectional = BidirType::Custom { request: req, response: resp };
                                }
                                _ => {
                                    return Err(syn::Error::new_spanned(
                                        &path,
                                        "bidirectional(...) requires both request and response parameters: bidirectional(request = \"MyReq\", response = \"MyResp\")"
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }

        // R-1 (AUTHZ-CORE-2 acceptance criterion #2): `public` and
        // `scope = "..."` together is a contradiction — a public method has
        // no required scope to enforce.
        if public && !scopes.is_empty() {
            let span = public_span
                .or(first_scope_span)
                .unwrap_or_else(proc_macro2::Span::call_site);
            return Err(syn::Error::new(
                span,
                "a public method has no required scope; remove `public` or remove `scope = ...`",
            ));
        }

        Ok(HubMethodAttrs { name, description, param_docs, bidirectional, http_method, request_override, public, scopes })
    }
}

/// Maximum word count for description
const MAX_DESCRIPTION_WORDS: usize = 15;

/// Parsed attributes for #[hub_methods]
pub struct HubMethodsAttrs {
    /// `namespace = "..."`, or `None` for the implicit namespace
    /// ([`implicit_namespace`] of the struct's name, RETURNS-8).
    pub namespace: Option<String>,
    /// Explicit version string, or None to use CARGO_PKG_VERSION at compile time
    pub version: Option<String>,
    /// Short description (max 15 words)
    pub description: Option<String>,
    /// Long description (optional, for detailed documentation)
    pub long_description: Option<String>,
    /// Explicit crate path override. `None` means the default resolver
    /// (`proc-macro-crate`) should pick the path at codegen time.
    pub crate_path: Option<String>,
    /// If true, generate resolve_handle that delegates to self.resolve_handle_impl()
    pub resolve_handle: bool,
    /// If true, this activation is a hub with children (calls self.plugin_children())
    pub hub: bool,
    /// IR-5: span of the `hub` flag, used to anchor the deprecation warning.
    pub hub_span: Option<proc_macro2::Span>,
    /// Stable UUID for this plugin instance (for handle routing)
    /// If not provided, a deterministic UUID is generated from namespace
    pub plugin_id: Option<String>,
    /// If set, namespace() will call this method instead of returning the constant
    /// e.g., namespace_fn = "runtime_namespace" generates: fn namespace(&self) -> &str { self.runtime_namespace() }
    pub namespace_fn: Option<String>,
    /// Activation-level request type (from `request = MyRequest`).
    /// When set, the dispatch code extracts this type from the raw HTTP context
    /// and injects fields into methods that declare `#[activation_param]`.
    pub request_type: Option<syn::Type>,
    /// Child activation field names (from `children = [field_a, field_b]`).
    /// When set, the generated `ChildRouter::get_child()` dispatches by field name,
    /// enabling type-safe `parent.child.method` hierarchical routing.
    /// Each ident must match both a struct field name and the child's namespace.
    pub children: Vec<syn::Ident>,
    /// IR-5: span of the `children = [...]` attribute, for deprecation warning.
    pub children_span: Option<proc_macro2::Span>,
    /// REQ-10: locked fields of the activation-level request struct.
    /// When non-empty, methods CANNOT use `#[plexus_macros::method(request = ())]`
    /// to drop activation-level extraction — the macro emits a compile error
    /// naming the locked fields. Field idents must be in the activation's
    /// request struct.
    pub required_fields: Vec<syn::Ident>,
    /// Span of the `required = [...]` attribute for diagnostics.
    pub required_fields_span: Option<proc_macro2::Span>,
    /// RED-6: declared auth posture for this activation.
    ///
    /// - `Required`: every method must have `#[from_auth]` OR
    ///   `#[plexus_macros::method(public)]` — compile error otherwise.
    /// - `Optional` (default): no enforcement; RED-2 emits a compile warning
    ///   when auth is asymmetric across methods.
    /// - `Mixed`: like `Optional`, but suppresses the RED-2 warning —
    ///   explicit acknowledgment that the asymmetry is intentional.
    /// - `None`: no method may have `#[from_auth]` — compile error if any
    ///   method does. Affirmative declaration of a public activation.
    ///
    /// Parsed from `auth = "required" | "optional" | "mixed" | "none"`.
    ///
    /// R-1 (joint AP-1): `None` means the activation never declared a
    /// posture — enforcement treats that as `Optional` (unchanged), but the
    /// IR emission stays silent so posture-silent activations produce
    /// byte-identical schemas to pre-R-1 (AP-1 acceptance criterion #2).
    /// `Some(p)` is emitted into every generated `MethodSchema.auth_posture`.
    pub auth_posture: Option<AuthPosture>,
    /// MACRO-2: emit the typed client alongside the server side.
    ///
    /// Opt-in (`client` as a bare flag) rather than automatic, because the
    /// emitted client links against `plexus-client`; turning it on for
    /// everyone would break every existing consumer that has not added the
    /// dependency. Same additive rule AUTHZ-CRED-MACRO-3 used for
    /// `plexus-auth-core`.
    pub client: bool,
    /// Span of the `client` flag, so a missing `plexus-client` dependency
    /// points at the flag that asked for it.
    pub client_span: Option<proc_macro2::Span>,
    /// MACRO-2: a `cfg` predicate under which the **server** half of the
    /// expansion is compiled — the impl block, the method enum, the
    /// `Activation` impl, the jsonrpsee trait, the dependency shim.
    ///
    /// This is what makes an activation crate reachable from wasm32. With
    /// `server_cfg = "feature = \"server\""`, a `--no-default-features`
    /// build of the activation's crate emits *only* the typed client, which
    /// depends on nothing but `plexus-client` and the activation's own
    /// event and parameter types. The consumer says one dependency line
    /// (`tickets-api = { version = "..", default-features = false }`) and
    /// gets a client that compiles for the browser.
    ///
    /// The predicate is any `cfg` meta: `feature = "server"`,
    /// `not(target_arch = "wasm32")`, `all(..)`. Only the macro's own output
    /// is gated — the author still gates their own struct definition and
    /// server-only imports, as they would for any optional dependency.
    pub server_cfg: Option<String>,
    /// Span of `server_cfg`, for diagnostics.
    pub server_cfg_span: Option<proc_macro2::Span>,
}

/// RED-6: declared auth enforcement posture for an activation.
/// See `HubMethodsAttrs::auth_posture` for semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AuthPosture {
    Required,
    #[default]
    Optional,
    Mixed,
    None,
}

impl Parse for HubMethodsAttrs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut namespace: Option<String> = None;
        let mut version: Option<String> = None;
        let mut description = None;
        let mut long_description = None;
        let mut crate_path: Option<String> = None;
        let mut resolve_handle = false;
        let mut hub = false;
        let mut hub_span: Option<proc_macro2::Span> = None;
        let mut plugin_id = None;
        let mut namespace_fn = None;
        let mut request_type: Option<syn::Type> = None;
        let mut children: Vec<syn::Ident> = Vec::new();
        let mut children_span: Option<proc_macro2::Span> = None;
        let mut required_fields: Vec<syn::Ident> = Vec::new();
        let mut required_fields_span: Option<proc_macro2::Span> = None;
        // R-1: `None` = posture never declared (enforced as Optional, not
        // emitted); `Some(p)` = declared and emitted.
        let mut auth_posture: Option<AuthPosture> = None;
        // MACRO-2: opt-in typed-client emission.
        let mut client = false;
        let mut client_span: Option<proc_macro2::Span> = None;
        let mut server_cfg: Option<String> = None;
        let mut server_cfg_span: Option<proc_macro2::Span> = None;

        if !input.is_empty() {
            let metas = Punctuated::<Meta, Token![,]>::parse_terminated(input)?;

            for meta in metas {
                match meta {
                    Meta::NameValue(MetaNameValue { path, value, .. }) => {
                        // Handle `request = SomeType` — value is a path/type expression,
                        // not a string literal. We parse the Expr and convert to a Type.
                        if path.is_ident("request") {
                            // The value after `=` is an expression (e.g. Expr::Path for `TestRequest`).
                            // Expr::Path can be directly reinterpreted as a Type::Path.
                            let ty: syn::Type = expr_to_type(&value).ok_or_else(|| {
                                syn::Error::new_spanned(&path, "request = <Type>: expected a type path (e.g. request = MyRequest or request = ())")
                            })?;
                            request_type = Some(ty);
                            continue;
                        }

                        if let Expr::Lit(ExprLit { lit: Lit::Str(s), .. }) = value {
                            if path.is_ident("namespace") {
                                namespace = Some(s.value());
                            } else if path.is_ident("version") {
                                version = Some(s.value());
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
                                crate_path = Some(s.value());
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
                            } else if path.is_ident("server_cfg") {
                                // MACRO-2: cfg predicate gating the server half.
                                server_cfg = Some(s.value());
                                server_cfg_span = Some(s.span());
                            } else if path.is_ident("auth") {
                                // RED-6: declared auth enforcement posture.
                                // See HubMethodsAttrs::auth_posture for semantics.
                                let v = s.value();
                                auth_posture = match v.as_str() {
                                    "required" => Some(AuthPosture::Required),
                                    "optional" => Some(AuthPosture::Optional),
                                    "mixed"    => Some(AuthPosture::Mixed),
                                    "none"     => Some(AuthPosture::None),
                                    other => return Err(syn::Error::new(
                                        s.span(),
                                        format!(
                                            "auth must be one of \"required\", \"optional\", \"mixed\", \"none\"; got {:?}",
                                            other,
                                        ),
                                    )),
                                };
                            }
                        }
                    }
                    Meta::Path(path) => {
                        // Handle bare flags like `resolve_handle`, `hub`
                        if path.is_ident("resolve_handle") {
                            resolve_handle = true;
                        } else if path.is_ident("hub") {
                            hub = true;
                            hub_span = Some(path.span());
                        } else if path.is_ident("client") {
                            // MACRO-2: also emit the typed client.
                            client = true;
                            client_span = Some(path.span());
                        }
                    }
                    Meta::List(list) => {
                        // children = [field_a, field_b, ...]
                        if list.path.is_ident("children") {
                            let parser = Punctuated::<syn::Ident, Token![,]>::parse_terminated;
                            let idents = syn::parse::Parser::parse2(parser, list.tokens.clone())?;
                            children = idents.into_iter().collect();
                            children_span = Some(list.path.span());
                        }
                        // REQ-10: required = [field_a, field_b, ...] — activation-level
                        // lock list. Methods may not drop these with `request = ()`.
                        else if list.path.is_ident("required") {
                            let parser = Punctuated::<syn::Ident, Token![,]>::parse_terminated;
                            let idents = syn::parse::Parser::parse2(parser, list.tokens.clone())?;
                            required_fields = idents.into_iter().collect();
                            required_fields_span = Some(list.path.span());
                        }
                    }
                }
            }
        }

        if namespace.as_deref() == Some("") {
            return Err(syn::Error::new(
                input.span(),
                "namespace = \"\" is empty; omit `namespace` for the implicit one \
                 (the struct's name in snake_case, a trailing `Hub` / `Activation` dropped)",
            ));
        }

        Ok(HubMethodsAttrs {
            namespace,
            version,
            description,
            long_description,
            crate_path,
            resolve_handle,
            hub,
            hub_span,
            plugin_id,
            namespace_fn,
            request_type,
            children,
            children_span,
            required_fields,
            required_fields_span,
            auth_posture,
            client,
            client_span,
            server_cfg,
            server_cfg_span,
        })
    }
}

/// Parameter info with optional description
pub struct ParamInfo {
    pub name: syn::Ident,
    pub ty: Type,
    pub description: Option<String>,
    /// IR-5: parsed `#[deprecated(...)]` (+ optional
    /// `#[plexus_macros::removed_in]`) on this parameter in the method
    /// signature. Populated into `ParamSchema.deprecation` on the emitted
    /// `MethodSchema.params_meta` list.
    pub deprecation: Option<ParsedDeprecation>,
}

/// A parameter injected from the activation's extracted request struct.
/// Created by `#[activation_param]` on a method parameter.
#[derive(Debug, Clone)]
pub struct ActivationParamInfo {
    /// The parameter name in the method signature (must match a field in the request struct)
    pub param_name: syn::Ident,
    /// The declared type in the method (Rust enforces this matches the request struct field type)
    pub ty: Type,
}

/// The kind of child-dispatch a `#[child]`-annotated method participates in.
#[derive(Debug, Clone)]
pub enum ChildMethodKind {
    /// `fn NAME(&self) -> Child` — a named static child whose routing name
    /// is the method identifier.
    Static,
    /// `fn NAME(&self, name: &str) -> Option<Child>` — a fallback dispatcher
    /// that receives every name not matched by a static child.
    Dynamic,
}

/// Information extracted from a `#[plexus_macros::child]`-annotated method.
#[derive(Debug, Clone)]
pub struct ChildMethodInfo {
    /// The function identifier (used as routing name for static children).
    pub fn_name: syn::Ident,
    /// Doc-comment-derived description. Currently unused for codegen but
    /// threaded through so future tickets (CHILD-4) can project it into
    /// child listings/schemas.
    #[allow(dead_code)]
    pub description: String,
    /// Whether this is a static (no-arg) or dynamic (name: &str) child method.
    pub kind: ChildMethodKind,
    /// Whether the method is `async fn` — the generated dispatcher awaits it.
    pub is_async: bool,
    /// CHILD-4: when `#[child(list = "METHOD")]` is present, the name of the
    /// sibling method that streams child names for `ChildRouter::list_children`.
    pub list_fn: Option<syn::Ident>,
    /// CHILD-4: when `#[child(search = "METHOD")]` is present, the name of the
    /// sibling method that streams child names matching a query for
    /// `ChildRouter::search_children`.
    pub search_fn: Option<syn::Ident>,
    /// IR-3: parsed `#[deprecated(...)]` + `#[plexus_macros::removed_in]` info
    /// folded into the emitted `MethodSchema.deprecation` field.
    pub deprecation: Option<ParsedDeprecation>,
}

impl ChildMethodInfo {
    /// Build a `ChildMethodInfo` from an impl-block method. Validates the
    /// signature and returns a `syn::Error` with the phrase
    /// `child method signature` for any unsupported shape.
    pub fn from_fn(method: &ImplItemFn) -> syn::Result<Self> {
        let fn_name = method.sig.ident.clone();
        let is_async = method.sig.asyncness.is_some();
        let description = extract_doc_description(&method.attrs).unwrap_or_default();

        // CHILD-4: parse `list = "method"` / `search = "method"` args off the
        // `#[child(...)]` attribute. The attribute itself is the sole carrier
        // of these args — no other attribute recognizes them.
        let (list_fn, search_fn) = parse_child_attr_args(&method.attrs)?;

        // IR-3: capture deprecation metadata from `#[deprecated(...)]` +
        // `#[plexus_macros::removed_in("X")]` on this child method.
        let deprecation = parse_deprecation_attrs(&method.attrs, method.sig.span())?;

        // Collect non-self typed parameters.
        let mut typed_params: Vec<&syn::PatType> = Vec::new();
        let mut has_receiver = false;
        for arg in &method.sig.inputs {
            match arg {
                FnArg::Receiver(_) => has_receiver = true,
                FnArg::Typed(pt) => typed_params.push(pt),
            }
        }

        if !has_receiver {
            return Err(syn::Error::new_spanned(
                &method.sig,
                "unsupported child method signature: a #[child] method must take `&self` \
                 (shapes: `fn NAME(&self) -> Child` or `fn NAME(&self, name: &str) -> Option<Child>`)",
            ));
        }

        let kind = match typed_params.len() {
            0 => ChildMethodKind::Static,
            1 => {
                // Must be a parameter of type `&str`.
                let pt = typed_params[0];
                if !is_str_ref(&pt.ty) {
                    return Err(syn::Error::new_spanned(
                        &pt.ty,
                        "unsupported child method signature: the single parameter of a dynamic \
                         #[child] method must be `name: &str` \
                         (shapes: `fn NAME(&self) -> Child` or `fn NAME(&self, name: &str) -> Option<Child>`)",
                    ));
                }
                ChildMethodKind::Dynamic
            }
            _ => {
                return Err(syn::Error::new_spanned(
                    &method.sig,
                    "unsupported child method signature: #[child] methods accept either \
                     no extra arguments (static child) or a single `name: &str` argument \
                     (dynamic child)",
                ));
            }
        };

        // Require an explicit return type; the specific trait bounds on the
        // return value (`ChildRouter + Clone + Send + Sync + 'static`) are
        // enforced by the generated code, not here — that gives better
        // error messages pointing at the user's type.
        if matches!(method.sig.output, ReturnType::Default) {
            return Err(syn::Error::new_spanned(
                &method.sig,
                "unsupported child method signature: a #[child] method must declare a return type \
                 (either `-> Child` for a static child or `-> Option<Child>` for a dynamic child)",
            ));
        }

        Ok(ChildMethodInfo {
            fn_name,
            description,
            kind,
            is_async,
            list_fn,
            search_fn,
            deprecation,
        })
    }
}

/// Parse the arguments of a `#[child(...)]` / `#[plexus_macros::child(...)]`
/// attribute. Supported args:
///
/// - `list = "method_name"` — links a sibling list method for CHILD-4
/// - `search = "method_name"` — links a sibling search method for CHILD-4
///
/// If the attribute is bare (`#[child]`) both returned values are `None`.
/// Unknown args are rejected so typos surface early.
fn parse_child_attr_args(
    attrs: &[syn::Attribute],
) -> syn::Result<(Option<syn::Ident>, Option<syn::Ident>)> {
    let mut list_fn: Option<syn::Ident> = None;
    let mut search_fn: Option<syn::Ident> = None;

    for attr in attrs {
        let path = attr.path();
        let last = path.segments.last().map(|s| s.ident.to_string());
        if !matches!(last.as_deref(), Some("child")) {
            continue;
        }

        match &attr.meta {
            // Bare `#[child]` — no args, nothing to parse.
            Meta::Path(_) => {}
            // `#[child(list = "...", search = "...")]`
            Meta::List(list) => {
                let metas = list.parse_args_with(
                    Punctuated::<Meta, Token![,]>::parse_terminated,
                )?;
                for meta in metas {
                    match meta {
                        Meta::NameValue(MetaNameValue { path, value, .. }) => {
                            let ident_key = match path.get_ident() {
                                Some(id) => id.to_string(),
                                None => {
                                    return Err(syn::Error::new_spanned(
                                        &path,
                                        "unknown argument to #[plexus_macros::child]; \
                                         expected `list = \"method\"` or `search = \"method\"`",
                                    ));
                                }
                            };
                            let s = match value {
                                Expr::Lit(ExprLit { lit: Lit::Str(ref s), .. }) => s.clone(),
                                _ => {
                                    return Err(syn::Error::new_spanned(
                                        &value,
                                        format!(
                                            "#[plexus_macros::child({} = \"...\")] expects a string literal naming a sibling method",
                                            ident_key
                                        ),
                                    ));
                                }
                            };
                            let method_ident = syn::Ident::new(&s.value(), s.span());
                            match ident_key.as_str() {
                                "list" => {
                                    if list_fn.is_some() {
                                        return Err(syn::Error::new_spanned(
                                            &s,
                                            "duplicate `list = \"...\"` argument on #[plexus_macros::child]",
                                        ));
                                    }
                                    list_fn = Some(method_ident);
                                }
                                "search" => {
                                    if search_fn.is_some() {
                                        return Err(syn::Error::new_spanned(
                                            &s,
                                            "duplicate `search = \"...\"` argument on #[plexus_macros::child]",
                                        ));
                                    }
                                    search_fn = Some(method_ident);
                                }
                                other => {
                                    return Err(syn::Error::new_spanned(
                                        &path,
                                        format!(
                                            "unknown argument `{}` to #[plexus_macros::child]; \
                                             expected `list = \"method\"` or `search = \"method\"`",
                                            other
                                        ),
                                    ));
                                }
                            }
                        }
                        other => {
                            return Err(syn::Error::new_spanned(
                                other,
                                "unknown argument to #[plexus_macros::child]; \
                                 expected `list = \"method\"` or `search = \"method\"`",
                            ));
                        }
                    }
                }
            }
            Meta::NameValue(_) => {
                return Err(syn::Error::new_spanned(
                    attr,
                    "#[plexus_macros::child = ...] form is not supported; \
                     use `#[plexus_macros::child]` or `#[plexus_macros::child(list = \"...\", search = \"...\")]`",
                ));
            }
        }
    }

    Ok((list_fn, search_fn))
}

/// The stream-return shape of a sibling `list` / `search` method referenced by
/// `#[child(list = "...")]` / `#[child(search = "...")]`.
#[derive(Debug, Clone, Copy)]
pub enum ListSearchReturnShape {
    /// `-> impl Stream<Item = String> [+ bounds...]`
    ImplStream,
    /// `-> BoxStream<'_, String>` (any lifetime; any path tail)
    BoxStream,
}

/// Information extracted from a sibling method referenced by
/// `#[child(list = "...")]` or `#[child(search = "...")]`.
#[derive(Debug, Clone)]
pub struct ListSearchMethodInfo {
    pub fn_name: syn::Ident,
    pub is_async: bool,
    #[allow(dead_code)]
    pub return_shape: ListSearchReturnShape,
}

/// Enum tagging which kind of list/search method signature we're validating.
#[derive(Debug, Clone, Copy)]
pub enum ListSearchKind {
    /// `(async) fn METHOD(&self) -> Stream<Item = String>` form.
    List,
    /// `(async) fn METHOD(&self, query: &str) -> Stream<Item = String>` form.
    Search,
}

/// Find a sibling method in `items` by name and validate that its signature
/// matches the expected `list` / `search` shape. Returns `ListSearchMethodInfo`
/// on success, or a `syn::Error` with one of:
///   - `"not found in impl"` when no method of that name exists.
///   - a signature-mismatch message when parameters / return type don't match.
pub fn find_and_validate_list_search_method(
    items: &[ImplItem],
    name: &syn::Ident,
    kind: ListSearchKind,
    attr_owner: &syn::Ident,
) -> syn::Result<ListSearchMethodInfo> {
    // Locate the named sibling method.
    let method: &ImplItemFn = items
        .iter()
        .find_map(|it| match it {
            ImplItem::Fn(f) if f.sig.ident == *name => Some(f),
            _ => None,
        })
        .ok_or_else(|| {
            syn::Error::new(
                name.span(),
                format!(
                    "method `{}` referenced by #[plexus_macros::child({} = \"{}\")] \
                     on `{}` not found in impl",
                    name,
                    match kind {
                        ListSearchKind::List => "list",
                        ListSearchKind::Search => "search",
                    },
                    name,
                    attr_owner,
                ),
            )
        })?;

    // Validate signature: `&self` + extra typed params.
    let mut has_receiver = false;
    let mut typed_params: Vec<&syn::PatType> = Vec::new();
    for arg in method.sig.inputs.iter() {
        match arg {
            FnArg::Receiver(_) => has_receiver = true,
            FnArg::Typed(pt) => typed_params.push(pt),
        }
    }
    if !has_receiver {
        return Err(syn::Error::new_spanned(
            &method.sig,
            format!(
                "child method signature mismatch: `{}` must take `&self` (expected shape: `(async) fn {}({}) -> impl Stream<Item = String>` or `-> BoxStream<'_, String>`)",
                name,
                name,
                match kind {
                    ListSearchKind::List => "&self",
                    ListSearchKind::Search => "&self, query: &str",
                },
            ),
        ));
    }
    match kind {
        ListSearchKind::List => {
            if !typed_params.is_empty() {
                return Err(syn::Error::new_spanned(
                    &method.sig,
                    format!(
                        "child method signature mismatch: list method `{}` must take only `&self` (got {} extra parameter{}); expected `(async) fn {}(&self) -> impl Stream<Item = String>` or `-> BoxStream<'_, String>`",
                        name,
                        typed_params.len(),
                        if typed_params.len() == 1 { "" } else { "s" },
                        name,
                    ),
                ));
            }
        }
        ListSearchKind::Search => {
            if typed_params.len() != 1 {
                return Err(syn::Error::new_spanned(
                    &method.sig,
                    format!(
                        "child method signature mismatch: search method `{}` must take `&self` and exactly one `query: &str` parameter (got {}); expected `(async) fn {}(&self, query: &str) -> impl Stream<Item = String>` or `-> BoxStream<'_, String>`",
                        name,
                        typed_params.len(),
                        name,
                    ),
                ));
            }
            if !is_str_ref(&typed_params[0].ty) {
                return Err(syn::Error::new_spanned(
                    &typed_params[0].ty,
                    format!(
                        "child method signature mismatch: the query parameter of search method `{}` must be `&str`",
                        name
                    ),
                ));
            }
        }
    }

    // Validate return type: accept either `impl Stream<Item = String> [+ bounds]`
    // or `BoxStream<'_, String>`.
    let return_ty = match &method.sig.output {
        ReturnType::Default => {
            return Err(syn::Error::new_spanned(
                &method.sig,
                format!(
                    "child method signature mismatch: `{}` must declare a return type of \
                     `impl Stream<Item = String>` or `BoxStream<'_, String>`",
                    name
                ),
            ));
        }
        ReturnType::Type(_, ty) => ty.as_ref(),
    };

    let return_shape = classify_string_stream_return(return_ty).ok_or_else(|| {
        syn::Error::new_spanned(
            return_ty,
            format!(
                "child method signature mismatch: `{}` must return `impl Stream<Item = String>` or `BoxStream<'_, String>`",
                name,
            ),
        )
    })?;

    Ok(ListSearchMethodInfo {
        fn_name: method.sig.ident.clone(),
        is_async: method.sig.asyncness.is_some(),
        return_shape,
    })
}

/// Classify a return type as one of the accepted `String`-stream shapes, or
/// `None` if neither. Recognized:
///   - `impl Stream<Item = String> [+ Bounds]` (any bound order)
///   - `BoxStream<'a, String>` / `BoxStream<'_, String>` (any path tail of
///     `BoxStream`, single or double angle-bracket form)
fn classify_string_stream_return(ty: &Type) -> Option<ListSearchReturnShape> {
    // Case 1: impl Stream<Item = String> [+ ...]
    if let Type::ImplTrait(impl_trait) = ty {
        for bound in &impl_trait.bounds {
            if let syn::TypeParamBound::Trait(tb) = bound {
                let last = tb.path.segments.last()?;
                if last.ident == "Stream" {
                    if let PathArguments::AngleBracketed(args) = &last.arguments {
                        for arg in &args.args {
                            if let GenericArgument::AssocType(at) = arg {
                                if at.ident == "Item" && type_is_string(&at.ty) {
                                    return Some(ListSearchReturnShape::ImplStream);
                                }
                            }
                        }
                    }
                }
            }
        }
        return None;
    }

    // Case 2: BoxStream<'_, String> — a path type whose last segment is
    // `BoxStream` with angle-bracketed args including `String` as the item.
    if let Type::Path(tp) = ty {
        let last = tp.path.segments.last()?;
        if last.ident == "BoxStream" {
            if let PathArguments::AngleBracketed(args) = &last.arguments {
                // Look for the `String` generic argument (second position,
                // after the lifetime — though we accept it wherever).
                for arg in &args.args {
                    if let GenericArgument::Type(inner) = arg {
                        if type_is_string(inner) {
                            return Some(ListSearchReturnShape::BoxStream);
                        }
                    }
                }
            }
        }
    }

    None
}

/// True if `ty` is plain `String` (no generics, any path tail of that name).
fn type_is_string(ty: &Type) -> bool {
    if let Type::Path(tp) = ty {
        if let Some(seg) = tp.path.segments.last() {
            return seg.ident == "String" && matches!(seg.arguments, PathArguments::None);
        }
    }
    false
}

/// True if `ty` is exactly `&str` (with any lifetime and no mutability).
fn is_str_ref(ty: &Type) -> bool {
    if let Type::Reference(r) = ty {
        if r.mutability.is_some() {
            return false;
        }
        if let Type::Path(tp) = &*r.elem {
            if let Some(seg) = tp.path.segments.last() {
                return seg.ident == "str" && tp.path.segments.len() == 1;
            }
        }
    }
    false
}

/// Extract `ParsedDeprecation` from an item's attributes by reading
/// `#[deprecated(...)]` and the companion `#[plexus_macros::removed_in("X")]`.
///
/// # Rules (per ticket IR-3)
///
/// | Attributes present | Result |
/// |---|---|
/// | None | `Ok(None)` |
/// | `#[deprecated]` (bare) | `Ok(Some(ParsedDeprecation { since: "", removed_in: "unspecified", message: "" }))` |
/// | `#[deprecated(since = "X", note = "Y")]` | `Ok(Some(ParsedDeprecation { since: "X", removed_in: "unspecified", message: "Y" }))` |
/// | `#[deprecated(since = "X", note = "Y")]` + `#[plexus_macros::removed_in("Z")]` | `Ok(Some(ParsedDeprecation { since: "X", removed_in: "Z", message: "Y" }))` |
/// | `#[deprecated(since = "X", note = "Y", removed_in = "Z")]` | `Ok(Some(ParsedDeprecation { since: "X", removed_in: "Z", message: "Y" }))` — rustc warns about the unknown key; we read it |
/// | `#[plexus_macros::removed_in("Z")]` alone | `Err(..)` — companion without `#[deprecated]` is a compile error |
///
/// The `item_span` argument drives the span of the "companion without
/// `#[deprecated]`" error.
pub fn parse_deprecation_attrs(
    attrs: &[syn::Attribute],
    item_span: proc_macro2::Span,
) -> syn::Result<Option<ParsedDeprecation>> {
    let deprecated_attr = attrs.iter().find(|a| a.path().is_ident("deprecated"));
    let removed_in_attr = attrs.iter().find(|a| {
        let segs = &a.path().segments;
        // Accept `#[removed_in(...)]` (imported directly) or the fully-qualified
        // `#[plexus_macros::removed_in(...)]` form.
        segs.last()
            .map(|s| s.ident == "removed_in")
            .unwrap_or(false)
    });

    // IR-5: the `#[plexus_macros::removed_in(..)]` proc macro, when applied
    // OUTSIDE `#[plexus_macros::activation]` on an impl block, expands FIRST
    // and consumes itself before the activation macro runs. To bridge the
    // gap, that proc macro also emits a `#[doc = "__plexus_removed_in:VERSION"]`
    // sentinel on the item. We scan for the sentinel as an equivalent
    // signal.
    let removed_in_from_sentinel: Option<String> = attrs
        .iter()
        .filter(|a| a.path().is_ident("doc"))
        .filter_map(|a| {
            if let Meta::NameValue(MetaNameValue { value, .. }) = &a.meta {
                if let Expr::Lit(ExprLit { lit: Lit::Str(s), .. }) = value {
                    let v = s.value();
                    if let Some(rest) = v.strip_prefix("__plexus_removed_in:") {
                        return Some(rest.to_string());
                    }
                }
            }
            None
        })
        .next();

    if deprecated_attr.is_none() && removed_in_attr.is_none() && removed_in_from_sentinel.is_none() {
        return Ok(None);
    }

    if deprecated_attr.is_none() {
        // companion attr present without its required partner
        let err_span = removed_in_attr
            .map(|a| a.span())
            .unwrap_or_else(proc_macro2::Span::call_site);
        return Err(syn::Error::new(
            err_span,
            "#[plexus_macros::removed_in(\"...\")] requires a companion \
             #[deprecated] attribute on the same item — `removed_in` by itself \
             has no meaning. Add `#[deprecated(since = \"...\", note = \"...\")]` \
             alongside, or remove the `#[removed_in]` attribute.",
        ));
    }

    // Parse the #[deprecated] attribute.
    let mut since = String::new();
    let mut message = String::new();
    let mut removed_in_from_deprecated: Option<String> = None;

    let deprecated_attr = deprecated_attr.unwrap();
    match &deprecated_attr.meta {
        // Bare `#[deprecated]` — no args.
        Meta::Path(_) => {}
        Meta::List(list) => {
            // Parse inner args. Each arg should be `key = "value"`.
            let inner: Punctuated<Meta, Token![,]> =
                list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;
            for meta in inner {
                if let Meta::NameValue(MetaNameValue { path, value, .. }) = meta {
                    if let Expr::Lit(ExprLit { lit: Lit::Str(s), .. }) = value {
                        if path.is_ident("since") {
                            since = s.value();
                        } else if path.is_ident("note") {
                            message = s.value();
                        } else if path.is_ident("removed_in") {
                            // rustc emits a warning about this unrecognized key,
                            // but we honor the author's intent by reading it.
                            removed_in_from_deprecated = Some(s.value());
                        }
                        // Any other keys are ignored — rustc will warn separately.
                    }
                }
            }
        }
        Meta::NameValue(_) => {
            // `#[deprecated = "msg"]` is the short form — rustc treats `msg`
            // as the note. We honor the same contract.
            if let Meta::NameValue(MetaNameValue { value, .. }) = &deprecated_attr.meta {
                if let Expr::Lit(ExprLit { lit: Lit::Str(s), .. }) = value {
                    message = s.value();
                }
            }
        }
    }

    // Parse the companion `#[plexus_macros::removed_in("X")]` if present.
    let mut removed_in_from_companion: Option<String> = None;
    if let Some(attr) = removed_in_attr {
        match &attr.meta {
            Meta::List(list) => {
                // Expect a single string literal inside the parens.
                let s: syn::LitStr = syn::parse2(list.tokens.clone()).map_err(|_| {
                    syn::Error::new_spanned(
                        &list.tokens,
                        "#[plexus_macros::removed_in(\"...\")] expects a single string \
                         literal argument (e.g. #[plexus_macros::removed_in(\"0.7\")])",
                    )
                })?;
                removed_in_from_companion = Some(s.value());
            }
            Meta::NameValue(MetaNameValue { value, .. }) => {
                // `#[plexus_macros::removed_in = "0.7"]` form.
                if let Expr::Lit(ExprLit { lit: Lit::Str(s), .. }) = value {
                    removed_in_from_companion = Some(s.value());
                } else {
                    return Err(syn::Error::new_spanned(
                        value,
                        "#[plexus_macros::removed_in = \"...\"] expects a string literal",
                    ));
                }
            }
            Meta::Path(_) => {
                return Err(syn::Error::new(
                    attr.span(),
                    "#[plexus_macros::removed_in] requires an argument (e.g. \
                     #[plexus_macros::removed_in(\"0.7\")])",
                ));
            }
        }
    }

    // Precedence: companion attribute > sentinel from outer-macro expansion
    // > inside-deprecated key > "unspecified" fallback. The sentinel carries
    // the same author intent as the companion; we prefer the real attribute
    // if both are somehow present.
    let removed_in = removed_in_from_companion
        .or(removed_in_from_sentinel)
        .or(removed_in_from_deprecated)
        .unwrap_or_else(|| DEPRECATION_REMOVED_IN_UNSPECIFIED.to_string());

    let _ = item_span; // reserved for future span-derived diagnostics
    Ok(Some(ParsedDeprecation {
        since,
        removed_in,
        message,
    }))
}

/// Return `true` if this method has a `#[child]` / `#[plexus_macros::child]`
/// attribute. Used by the `#[activation]` driver to split child methods from
/// regular `#[method]` ones before per-method parsing.
pub fn has_child_attr(method: &ImplItemFn) -> bool {
    method.attrs.iter().any(|a| {
        let path = a.path();
        path.segments
            .last()
            .map(|s| s.ident == "child")
            .unwrap_or(false)
    })
}

/// Return `true` if this method has a `#[method]` / `#[hub_method]` /
/// `#[plexus_macros::method]` attribute.
pub fn has_method_attr(method: &ImplItemFn) -> bool {
    method.attrs.iter().any(|a| {
        let last = a.path().segments.last().map(|s| s.ident.to_string());
        matches!(last.as_deref(), Some("method") | Some("hub_method"))
    })
}

/// Information extracted from a #[hub_method] function
pub struct MethodInfo {
    pub fn_name: syn::Ident,
    pub method_name: String,
    pub description: String,
    pub params: Vec<ParamInfo>,
    /// RETURNS-8: one item (`-> T`, `-> Result<T, E>`) or many (`-> impl
    /// Stream<Item = …>`), read from the return type.
    pub cardinality: Cardinality,
    /// RETURNS-8: `T` — the item minus its `Result` / `Progress` wrappers.
    pub ok_ty: Type,
    /// RETURNS-8: `E` when the item is a `Result<_, E>`; `None` when the method
    /// is infallible.
    pub err_ty: Option<Type>,
    /// RETURNS-8: `P` when the item is a `Progress<_, P>` (or `Progress<_>`,
    /// whose `P` is the default `Note`); `None` when the method reports no
    /// progress.
    pub progress_ty: Option<ProgressTy>,
    /// Bidirectional channel type (from attribute or inferred from parameter type)
    pub bidirectional: BidirType,
    /// HTTP method for REST endpoints (GET, POST, PUT, DELETE, PATCH)
    /// None defaults to POST
    pub http_method: Option<String>,
    /// True if method requires auth context (detected from `auth: &AuthContext`
    /// or `auth: Option<&AuthContext>` parameter, or `#[from_auth(...)]` use).
    ///
    /// AUTHZ-MACRO-OPTIONAL-AUTH-1: this stays true for both shapes; the
    /// distinction lives in `auth_optional` below. Codegen uses both flags
    /// together to decide between `auth.ok_or_else(...)?` (force) and a
    /// straight `auth` pass-through (optional).
    pub requires_auth: bool,
    /// True iff the method declared `auth: Option<&AuthContext>` (vs.
    /// `auth: &AuthContext`). Meaningful only when `requires_auth` is true.
    ///
    /// AUTHZ-MACRO-OPTIONAL-AUTH-1: when set, the dispatch wrapper passes the
    /// raw `Option<&AuthContext>` through to the handler instead of
    /// unwrapping it.
    pub auth_optional: bool,
    /// Parameters resolved from auth context via `#[from_auth(resolver)]`
    pub auth_resolvers: Vec<AuthResolver>,
    /// Parameters injected from the activation's request struct via `#[activation_param]`
    /// These are stripped from RPC schema and injected at dispatch time from `req.field_name`
    pub activation_params: Vec<ActivationParamInfo>,
    /// Per-method request override. `Some(Skip)` means `request = ()` was set on this method,
    /// which causes the dispatch wrapper to skip activation-level request extraction.
    pub request_override: Option<MethodRequestOverride>,
    /// RED-2: explicit-public flag (mirrored from `HubMethodAttrs::public`).
    /// `true` when `#[plexus_macros::method(public)]` is set; suppresses the
    /// mixed-auth diagnostic for this method.
    ///
    /// R-1: also emitted into the generated `MethodSchema.public` flag.
    pub public: bool,
    /// R-1 (AUTHZ-CORE-2): explicit required scopes (mirrored from
    /// `HubMethodAttrs::scopes`). Emitted into the generated
    /// `MethodSchema.requires_credential` via
    /// `RequiredCredential::from_method_scope(s)` (single) /
    /// `from_method_scopes` (conjunction). Empty = implicit
    /// `<activation>.<method>` rule; nothing is emitted (the registry
    /// resolves implicit scopes at build time, AUTHZ-CORE-4).
    pub scopes: Vec<String>,
    /// IR-3: parsed `#[deprecated(...)]` + `#[plexus_macros::removed_in]` info
    /// folded into the emitted `MethodSchema.deprecation` field.
    pub deprecation: Option<ParsedDeprecation>,
}

impl MethodInfo {
    /// Extract method info from an impl function with optional hub_method attrs
    pub fn from_fn(method: &ImplItemFn, hub_method_attrs: Option<&HubMethodAttrs>) -> syn::Result<Self> {
        let fn_name = method.sig.ident.clone();
        let method_name = hub_method_attrs
            .and_then(|a| a.name.clone())
            .unwrap_or_else(|| fn_name.to_string());

        // Resolve description with this precedence:
        //   1. Explicit `description = "..."` on #[plexus_macros::method(...)] wins.
        //   2. Otherwise fall back to `///` doc comments on the method, joined with '\n'
        //      and with common leading whitespace stripped (same rule as `cargo doc`).
        //   3. If neither is present, description is the empty string.
        let description = match hub_method_attrs.and_then(|a| a.description.clone()) {
            Some(explicit) => explicit,
            None => extract_doc_description(&method.attrs).unwrap_or_default(),
        };

        // Get param docs from hub_method attrs
        let param_docs = hub_method_attrs
            .map(|a| &a.param_docs)
            .cloned()
            .unwrap_or_default();
        let http_method = hub_method_attrs
            .and_then(|a| a.http_method.clone());
        let request_override = hub_method_attrs
            .and_then(|a| a.request_override.clone());

        // Get bidirectional from attribute (may be overridden by parameter type inference)
        let mut bidirectional = hub_method_attrs
            .map(|a| a.bidirectional.clone())
            .unwrap_or(BidirType::None);

        // Track if method requires auth
        let mut requires_auth = false;
        // AUTHZ-MACRO-OPTIONAL-AUTH-1: track whether the user wrote
        // `auth: Option<&AuthContext>` (optional) vs `auth: &AuthContext`
        // (force-required). Defaults to false; gets flipped on only if the
        // optional shape is detected below.
        let mut auth_optional = false;
        let mut auth_resolvers: Vec<AuthResolver> = Vec::new();
        let mut activation_params: Vec<ActivationParamInfo> = Vec::new();

        // Extract parameters after &self
        let mut params = Vec::new();
        for arg in &method.sig.inputs {
            if let FnArg::Typed(pat_type) = arg {
                if let Pat::Ident(ident) = &*pat_type.pat {
                    let name = ident.ident.clone();
                    let name_str = name.to_string();

                    // Check for #[activation_param] attribute first
                    // e.g. #[activation_param] auth_token: String
                    if let Some(err) = check_activation_param_attr(&pat_type.attrs)? {
                        return Err(err);
                    }
                    if has_activation_param_attr(&pat_type.attrs) {
                        activation_params.push(ActivationParamInfo {
                            param_name: name,
                            ty: (*pat_type.ty).clone(),
                        });
                        // Don't include in RPC params or schema
                        continue;
                    }

                    // Check for #[from_auth(resolver_expr)] attribute
                    // e.g. #[from_auth(self.db.validate_user)] user: ValidUser
                    if let Some(resolver) = extract_from_auth_attr(&pat_type.attrs) {
                        requires_auth = true;
                        auth_resolvers.push(AuthResolver {
                            param_name: name,
                            resolver_expr: resolver,
                        });
                        // Don't include in RPC params
                        continue;
                    }

                    // Check if this is an auth context parameter
                    // Detect auth: &AuthContext or auth: Option<&AuthContext>
                    if name_str == "auth" {
                        if let Some(shape) = classify_auth_context_type(&pat_type.ty) {
                            requires_auth = true;
                            // AUTHZ-MACRO-OPTIONAL-AUTH-1: remember which shape
                            // the user wrote so codegen emits the right
                            // unwrap/pass-through behavior.
                            auth_optional = matches!(shape, AuthParamShape::Optional);
                            // Don't include auth in params (it's provided by framework)
                            continue;
                        }
                    }

                    // Check if this is a bidirectional context parameter
                    // Detect ctx: &BidirChannel<Req, Resp> or ctx: &StandardBidirChannel
                    if name_str == "ctx" || name_str == "context" {
                        if let Some(bidir_types) = extract_bidir_channel_types(&pat_type.ty) {
                            // Infer bidirectional type from parameter
                            bidirectional = bidir_types;
                            // Don't include ctx in params (it's provided by framework)
                            continue;
                        }
                    }

                    // Param description precedence:
                    //   1. Explicit `params(name = "...")` on the method attribute wins.
                    //   2. Otherwise fall back to `///` doc comments on the parameter
                    //      itself (e.g. `async fn foo(&self, /// the input message
                    //      message: String) -> ...`). Mirrors the function-level
                    //      doc-fallback already implemented above.
                    //   3. If neither is present, description is `None`.
                    let description = param_docs
                        .get(&name_str)
                        .cloned()
                        .or_else(|| extract_doc_description(&pat_type.attrs));
                    // IR-5: parse `#[deprecated(...)]` + optional
                    // `#[plexus_macros::removed_in("...")]` on the parameter.
                    let deprecation =
                        parse_deprecation_attrs(&pat_type.attrs, pat_type.span())?;
                    params.push(ParamInfo {
                        name,
                        ty: (*pat_type.ty).clone(),
                        description,
                        deprecation,
                    });
                }
            }
        }

        // RETURNS-8: the return type is the method's whole contract —
        // cardinality, item, error and progress, read syntactically.
        let ReturnShape { cardinality, ok_ty, err_ty, progress_ty } =
            classify_return(&method.sig)?;

        // IR-3: extract deprecation info from the method's attributes (before
        // codegen strips the `#[plexus_macros::removed_in]` companion).
        let deprecation = parse_deprecation_attrs(&method.attrs, method.sig.span())?;

        // RED-2: capture the `public` flag from the per-method attribute,
        // if present. Default false.
        let public = hub_method_attrs.map(|a| a.public).unwrap_or(false);

        // R-1: capture the explicit `scope = "..."` literals (already
        // grammar-validated at attribute-parse time).
        let scopes = hub_method_attrs
            .map(|a| a.scopes.clone())
            .unwrap_or_default();

        Ok(MethodInfo {
            fn_name,
            method_name,
            description,
            params,
            cardinality,
            ok_ty,
            err_ty,
            progress_ty,
            bidirectional,
            http_method,
            requires_auth,
            auth_optional,
            auth_resolvers,
            activation_params,
            request_override,
            public,
            scopes,
            deprecation,
        })
    }
}

/// Convert a `syn::Expr` to a `syn::Type` for `request = Type` parsing.
///
/// Handles:
/// - `Expr::Path` (e.g. `TestRequest`, `my_mod::MyRequest`) → `Type::Path`
/// - `Expr::Tuple` with 0 elements (e.g. `()`) → `Type::Tuple` (unit type)
fn expr_to_type(expr: &Expr) -> Option<syn::Type> {
    match expr {
        Expr::Path(ExprPath { attrs: _, qself, path }) => {
            Some(syn::Type::Path(syn::TypePath {
                qself: qself.as_ref().map(|qs| syn::QSelf {
                    lt_token: qs.lt_token,
                    ty: qs.ty.clone(),
                    position: qs.position,
                    as_token: qs.as_token,
                    gt_token: qs.gt_token,
                }),
                path: path.clone(),
            }))
        }
        Expr::Tuple(ExprTuple { elems, .. }) if elems.is_empty() => {
            // `()` — unit type used for `request = ()` override
            Some(syn::parse_quote! { () })
        }
        _ => None,
    }
}

/// Check if a parameter has the `#[activation_param]` attribute.
/// Returns `true` if the bare `#[activation_param]` attribute is present.
fn has_activation_param_attr(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| attr.path().is_ident("activation_param"))
}

/// Validate that `#[activation_param]` takes no arguments.
/// Returns `Some(Err(...))` if the attribute is present but has arguments,
/// `None` if the attribute is absent or is correctly bare.
fn check_activation_param_attr(attrs: &[syn::Attribute]) -> syn::Result<Option<syn::Error>> {
    for attr in attrs {
        if attr.path().is_ident("activation_param") {
            // Make sure it's a bare path attribute, not #[activation_param(...)]
            if let syn::Meta::List(list) = &attr.meta {
                return Ok(Some(syn::Error::new_spanned(
                    list,
                    "activation_param takes no arguments — extraction is defined on the request struct, not the method",
                )));
            }
        }
    }
    Ok(None)
}

/// Extract the resolver expression from a `#[from_auth(expr)]` attribute.
/// Returns `Some(expr)` if the attribute is found, `None` otherwise.
///
/// Supports:
/// - `#[from_auth(self.db.validate_user)]`
/// - `#[from_auth(self.require_admin)]`
/// - Any arbitrary expression that will be called with `&AuthContext`
fn extract_from_auth_attr(attrs: &[syn::Attribute]) -> Option<syn::Expr> {
    for attr in attrs {
        if attr.path().is_ident("from_auth") {
            // Parse the inner expression from #[from_auth(expr)]
            let expr: syn::Expr = attr.parse_args().ok()?;
            return Some(expr);
        }
    }
    None
}

/// Classify an auth-context parameter type as either force-required
/// (`&AuthContext`) or optional (`Option<&AuthContext>`).
///
/// Returns `None` if `ty` is not an auth-context shape we recognize.
///
/// AUTHZ-MACRO-OPTIONAL-AUTH-1: split out from `is_auth_context_type` so that
/// downstream codegen can emit `auth.ok_or_else(...)?` for force-required
/// handlers and a plain `auth` pass-through for optional handlers.
pub(crate) fn classify_auth_context_type(ty: &Type) -> Option<AuthParamShape> {
    // Handle &AuthContext
    if let Type::Reference(type_ref) = ty {
        if let Type::Path(type_path) = &*type_ref.elem {
            if let Some(last_segment) = type_path.path.segments.last() {
                if last_segment.ident == "AuthContext" {
                    return Some(AuthParamShape::Required);
                }
            }
        }
    }

    // Handle Option<&AuthContext>
    if let Type::Path(type_path) = ty {
        if let Some(last_segment) = type_path.path.segments.last() {
            if last_segment.ident == "Option" {
                if let PathArguments::AngleBracketed(args) = &last_segment.arguments {
                    if let Some(GenericArgument::Type(inner_ty)) = args.args.first() {
                        // Option<&AuthContext> — inner must be the reference form
                        if classify_auth_context_type(inner_ty)
                            == Some(AuthParamShape::Required)
                        {
                            return Some(AuthParamShape::Optional);
                        }
                    }
                }
            }
        }
    }

    None
}

/// Shape of the `auth: ...` parameter on a hub method.
///
/// `Required` means the user wrote `auth: &AuthContext` and missing context
/// should produce an `Unauthenticated` error. `Optional` means the user
/// wrote `auth: Option<&AuthContext>` and missing context should be passed
/// through as `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthParamShape {
    Required,
    Optional,
}

/// Extract BidirChannel type parameters from a type like &BidirChannel<Req, Resp>
/// Handles various forms:
/// - &BidirChannel<Req, Resp>
/// - &Arc<BidirChannel<Req, Resp>>
/// - &StandardBidirChannel
/// - &Arc<StandardBidirChannel>
/// Returns BidirType::Custom if custom types found, BidirType::Standard if StandardBidirChannel
fn extract_bidir_channel_types(ty: &Type) -> Option<BidirType> {
    // Handle &T
    let inner_ty = if let Type::Reference(type_ref) = ty {
        &*type_ref.elem
    } else {
        ty
    };

    if let Type::Path(type_path) = inner_ty {
        let last_segment = type_path.path.segments.last()?;

        // Check for Arc<...> wrapper
        if last_segment.ident == "Arc" {
            if let PathArguments::AngleBracketed(args) = &last_segment.arguments {
                if let Some(GenericArgument::Type(inner)) = args.args.first() {
                    // Recursively check the inner type
                    return extract_bidir_from_path(inner);
                }
            }
            return None;
        }

        // Check direct type (no Arc)
        return extract_bidir_from_path(inner_ty);
    }

    None
}

/// Extract BidirType from a path type (BidirChannel or StandardBidirChannel)
fn extract_bidir_from_path(ty: &Type) -> Option<BidirType> {
    if let Type::Path(type_path) = ty {
        let last_segment = type_path.path.segments.last()?;

        // Check for StandardBidirChannel (type alias)
        if last_segment.ident == "StandardBidirChannel" {
            return Some(BidirType::Standard);
        }

        // Check for BidirChannel<Req, Resp>
        if last_segment.ident == "BidirChannel" {
            if let PathArguments::AngleBracketed(args) = &last_segment.arguments {
                let args_vec: Vec<_> = args.args.iter().collect();
                if args_vec.len() == 2 {
                    // Extract Req and Resp type names
                    if let (GenericArgument::Type(req_ty), GenericArgument::Type(resp_ty)) =
                        (args_vec[0], args_vec[1])
                    {
                        let req_name = type_to_string(req_ty);
                        let resp_name = type_to_string(resp_ty);

                        // Check if it's StandardRequest/StandardResponse
                        if req_name == "StandardRequest" && resp_name == "StandardResponse" {
                            return Some(BidirType::Standard);
                        }

                        return Some(BidirType::Custom {
                            request: req_name,
                            response: resp_name,
                        });
                    }
                }
            }
        }
    }

    None
}

/// Convert a Type to its string representation (simple path types only)
fn type_to_string(ty: &Type) -> String {
    if let Type::Path(type_path) = ty {
        type_path
            .path
            .segments
            .iter()
            .map(|s| s.ident.to_string())
            .collect::<Vec<_>>()
            .join("::")
    } else {
        format!("{:?}", ty)
    }
}

// ============================================================================
// RETURNS-8: the signature forms
// ============================================================================

/// Whether a method yields one item or many (RETURNS-1 decision 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cardinality {
    /// `-> T` / `-> Result<T, E>`: exactly one item.
    One,
    /// `-> impl Stream<Item = …>`: zero or more items.
    Many,
}

/// The progress type `P` of a `Progress<T, P>` item.
#[derive(Debug, Clone)]
pub enum ProgressTy {
    /// `Progress<T>`: `P` is the default, `plexus_client::Note`. It has no
    /// path of its own in the signature, so each half of the expansion names
    /// it through the crate it links — the server through `plexus-core`'s
    /// re-export, a client through `plexus-client` (see [`ProgressTy::to_type`]).
    Note,
    /// `Progress<T, P>`: `P` as written.
    Declared(Type),
}

impl ProgressTy {
    /// `P` as a type, given the path of a crate that exports `Note` at
    /// `<vocabulary>::Note` (`::plexus_core::plexus` server-side,
    /// `::plexus_client` client-side).
    pub fn to_type(&self, vocabulary: &syn::Path) -> Type {
        match self {
            ProgressTy::Note => syn::parse_quote!(#vocabulary::Note),
            ProgressTy::Declared(ty) => ty.clone(),
        }
    }
}

/// A method's return type, taken apart (RETURNS-1 decisions 2, 3, 10).
pub struct ReturnShape {
    pub cardinality: Cardinality,
    pub ok_ty: Type,
    pub err_ty: Option<Type>,
    pub progress_ty: Option<ProgressTy>,
}

/// The six forms, as the error that names them lists them.
const SIX_FORMS: &str = "\
    -> T\n    \
    -> Result<T, E>\n    \
    -> impl Stream<Item = T>\n    \
    -> impl Stream<Item = Result<T, E>>\n    \
    -> impl Stream<Item = Progress<T, P>>\n    \
    -> impl Stream<Item = Result<Progress<T, P>, E>>";

fn not_a_form(span: impl quote::ToTokens, method: &syn::Ident) -> syn::Error {
    syn::Error::new_spanned(
        span,
        format!(
            "#[plexus_macros::method] `{method}`: the return type is not one of the six \
             signature forms:\n    {SIX_FORMS}\n\
             A boxed or named stream (`BoxStream`, `Pin<Box<dyn Stream>>`, `PlexusStream`) is \
             not recognised — return `impl Stream<Item = ...> + Send + 'static` instead. \
             `Result` (two type arguments) and `Progress` (one or two) are recognised by \
             name; an alias of either is a plain `T`."
        ),
    )
}

/// Read a method's signature form (RETURNS-1 pin 14: syntactic — the last
/// path segment `Result` with two type arguments, `Progress` with one or
/// two; `impl Stream<Item = X>` with any extra bounds).
pub fn classify_return(sig: &syn::Signature) -> syn::Result<ReturnShape> {
    let method = &sig.ident;
    let ty = match &sig.output {
        ReturnType::Default => return Err(not_a_form(sig, method)),
        ReturnType::Type(_, ty) => ungroup(ty),
    };

    let (item, cardinality) = match ty {
        Type::ImplTrait(it) => (
            impl_stream_item(it).ok_or_else(|| not_a_form(ty, method))?,
            Cardinality::Many,
        ),
        _ => (ty, Cardinality::One),
    };

    let (inner, err_ty) = match result_args(item) {
        Some((t, e)) => (t, Some(e)),
        None => (item, None),
    };
    let (ok_ty, progress_ty) = match progress_args(inner) {
        Some((t, p)) => (t, Some(p)),
        None => (inner, None),
    };

    if progress_ty.is_some() && cardinality == Cardinality::One {
        return Err(syn::Error::new_spanned(
            ty,
            format!(
                "#[plexus_macros::method] `{method}`: a method that reports progress yields \
                 many items — return `impl Stream<Item = Progress<T, P>>` (or \
                 `impl Stream<Item = Result<Progress<T, P>, E>>`). A one-item method \
                 cannot report progress."
            ),
        ));
    }

    for part in [Some(ok_ty), err_ty].into_iter().flatten() {
        if !is_plain_item(part) {
            return Err(not_a_form(part, method));
        }
    }
    if let Some(ProgressTy::Declared(p)) = &progress_ty {
        if !is_plain_item(p) {
            return Err(not_a_form(p, method));
        }
    }

    Ok(ReturnShape {
        cardinality,
        ok_ty: ok_ty.clone(),
        err_ty: err_ty.cloned(),
        progress_ty,
    })
}

/// `(T)` and invisible groups are the type inside them.
fn ungroup(ty: &Type) -> &Type {
    match ty {
        Type::Paren(p) => ungroup(&p.elem),
        Type::Group(g) => ungroup(&g.elem),
        other => other,
    }
}

/// `X` of `impl Stream<Item = X> [+ bounds]`; `None` for any other `impl`.
fn impl_stream_item(it: &syn::TypeImplTrait) -> Option<&Type> {
    it.bounds.iter().find_map(|bound| {
        let syn::TypeParamBound::Trait(tb) = bound else { return None };
        let last = tb.path.segments.last()?;
        if last.ident != "Stream" {
            return None;
        }
        let PathArguments::AngleBracketed(args) = &last.arguments else { return None };
        args.args.iter().find_map(|arg| match arg {
            GenericArgument::AssocType(at) if at.ident == "Item" => Some(ungroup(&at.ty)),
            _ => None,
        })
    })
}

/// The type arguments of a path type's last segment, if it is `name`.
fn type_args<'a>(ty: &'a Type, name: &str) -> Option<Vec<&'a Type>> {
    let Type::Path(tp) = ty else { return None };
    if tp.qself.is_some() {
        return None;
    }
    let last = tp.path.segments.last()?;
    if last.ident != name {
        return None;
    }
    let PathArguments::AngleBracketed(args) = &last.arguments else { return None };
    let tys: Vec<&Type> = args
        .args
        .iter()
        .filter_map(|a| match a {
            GenericArgument::Type(t) => Some(ungroup(t)),
            _ => None,
        })
        .collect();
    (tys.len() == args.args.len()).then_some(tys)
}

/// `(T, E)` of `Result<T, E>` — exactly two type arguments.
fn result_args(ty: &Type) -> Option<(&Type, &Type)> {
    match type_args(ty, "Result")?.as_slice() {
        [t, e] => Some((t, e)),
        _ => None,
    }
}

/// `(T, P)` of `Progress<T, P>` / `Progress<T>` (`P` = `Note`).
fn progress_args(ty: &Type) -> Option<(&Type, ProgressTy)> {
    match type_args(ty, "Progress")?.as_slice() {
        [t] => Some((t, ProgressTy::Note)),
        [t, p] => Some((t, ProgressTy::Declared((*p).clone()))),
        _ => None,
    }
}

/// `false` for a type that is a stream or an opaque trait rather than a
/// value: `impl Trait`, `dyn Trait`, `Box<dyn …>`, `Pin<…>`, and the named
/// stream types (`BoxStream`, `LocalBoxStream`, `PlexusStream`, `Stream`).
/// Any other type is a plain item the schema and the wire can carry.
fn is_plain_item(ty: &Type) -> bool {
    match ungroup(ty) {
        Type::ImplTrait(_) | Type::TraitObject(_) => false,
        Type::Path(tp) => {
            let Some(last) = tp.path.segments.last() else { return true };
            let name = last.ident.to_string();
            if matches!(
                name.as_str(),
                "BoxStream" | "LocalBoxStream" | "PlexusStream" | "Stream" | "Pin"
            ) {
                return false;
            }
            if name == "Box" {
                if let Some(args) = type_args(ty, "Box") {
                    return !args.iter().any(|t| matches!(t, Type::TraitObject(_)));
                }
            }
            true
        }
        _ => true,
    }
}

/// RETURNS-8: the implicit namespace (RETURNS-1 decision 7) — the struct's
/// name in snake_case, with one trailing `Hub` or `Activation` removed:
/// `Tickets` → `tickets`, `HyperforgeHub` → `hyperforge`, `InfoActivation`
/// → `info`, `ClaudeCode` → `claude_code`. A struct named exactly `Hub` or
/// `Activation` keeps its name.
pub fn implicit_namespace(struct_name: &str) -> String {
    let base = ["Activation", "Hub"]
        .iter()
        .find_map(|suffix| struct_name.strip_suffix(suffix).filter(|rest| !rest.is_empty()))
        .unwrap_or(struct_name);
    to_snake_case(base)
}

/// `ClaudeCode` → `claude_code`, `HTTPServer` → `http_server`, `V2Api` →
/// `v2_api`: a word starts at an uppercase letter after a lowercase letter
/// or digit, and at the last capital of an acronym followed by lowercase.
fn to_snake_case(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len() + 4);
    for (i, &c) in chars.iter().enumerate() {
        if c.is_uppercase() {
            let boundary = match i.checked_sub(1).map(|j| chars[j]) {
                Some(prev) if prev.is_lowercase() || prev.is_ascii_digit() => true,
                Some(prev) if prev.is_uppercase() => {
                    chars.get(i + 1).is_some_and(|next| next.is_lowercase())
                }
                _ => false,
            };
            if boundary {
                out.push('_');
            }
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// Extract the description from `///` doc comment attributes on an item.
///
/// In Rust's AST both `///` doc-comment sugar and the raw `#[doc = "..."]` form appear
/// as `#[doc = "..."]` attributes, so we read them uniformly here.
///
/// Returns:
/// - `Some(String)` with lines joined by `\n` and common leading whitespace stripped,
///   matching the standard `cargo doc` rule, when at least one `#[doc = "..."]`
///   attribute is present.
/// - `None` when no doc-comment attributes are present on the item. Callers should
///   treat this as "no default available" and fall back to whatever behavior the
///   absence of a description should yield (typically the empty string).
pub(crate) fn extract_doc_description(attrs: &[syn::Attribute]) -> Option<String> {
    let mut raw_lines: Vec<String> = Vec::new();
    for attr in attrs {
        if attr.path().is_ident("doc") {
            if let Meta::NameValue(MetaNameValue { value, .. }) = &attr.meta {
                if let Expr::Lit(ExprLit { lit: Lit::Str(s), .. }) = value {
                    // `///` sugar prepends a single leading space to the literal
                    // (e.g. `/// Foo` becomes `#[doc = " Foo"]`). The common-leading-
                    // whitespace strip below removes that uniformly. For multi-line
                    // raw `#[doc = "a\nb"]` forms, split on '\n' and treat each line
                    // independently so the strip rule applies per line, not per attr.
                    for line in s.value().split('\n') {
                        raw_lines.push(line.to_string());
                    }
                }
            }
        }
    }
    if raw_lines.is_empty() {
        return None;
    }
    Some(strip_common_leading_whitespace(&raw_lines))
}

/// Strip the common leading-whitespace prefix from every line (the `cargo doc` rule).
///
/// - Empty / whitespace-only lines are ignored when computing the common prefix and
///   emitted as empty strings in the output.
/// - The minimum count of leading whitespace characters across all non-empty lines
///   is the common prefix length; that many chars are removed from each non-empty line.
fn strip_common_leading_whitespace(lines: &[String]) -> String {
    let min_indent = lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.chars().take_while(|c| c.is_whitespace()).count())
        .min()
        .unwrap_or(0);
    lines
        .iter()
        .map(|l| {
            if l.trim().is_empty() {
                String::new()
            } else {
                // Skip the first `min_indent` chars (all whitespace by construction).
                l.chars().skip(min_indent).collect::<String>()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use proc_macro2::Span;
    use syn::parse_quote;

    /// Drives `parse_deprecation_attrs` on an inline `removed_in` key inside
    /// `#[deprecated(...)]`. Stable rustc emits `E0541` for this form, so
    /// the happy-path integration test in `tests/ir3_role_and_deprecation_tests.rs`
    /// cannot exercise it end-to-end. This unit test runs the parser directly
    /// on a synthetic attribute — no rustc attribute-check is involved.
    #[test]
    fn parse_deprecation_reads_inline_removed_in_key() {
        let attrs: Vec<syn::Attribute> =
            vec![parse_quote! { #[deprecated(since = "0.5", note = "use bar", removed_in = "0.7")] }];
        let parsed = parse_deprecation_attrs(&attrs, Span::call_site())
            .expect("parser accepts unknown key")
            .expect("produces Some for a #[deprecated] attr");
        assert_eq!(parsed.since, "0.5");
        assert_eq!(parsed.removed_in, "0.7");
        assert_eq!(parsed.message, "use bar");
    }

    /// The companion `#[plexus_macros::removed_in(...)]` takes precedence over
    /// a `removed_in` key written inside `#[deprecated(...)]`.
    #[test]
    fn parse_deprecation_companion_overrides_inline_key() {
        let attrs: Vec<syn::Attribute> = vec![
            parse_quote! { #[deprecated(since = "0.5", note = "n", removed_in = "0.7")] },
            parse_quote! { #[plexus_macros::removed_in("0.9")] },
        ];
        let parsed = parse_deprecation_attrs(&attrs, Span::call_site()).unwrap().unwrap();
        assert_eq!(parsed.removed_in, "0.9");
    }

    /// `#[deprecated]` without `removed_in` (neither inline nor companion)
    /// yields the documented fallback string `"unspecified"`.
    #[test]
    fn parse_deprecation_fallback_is_unspecified() {
        let attrs: Vec<syn::Attribute> =
            vec![parse_quote! { #[deprecated(since = "0.5", note = "n")] }];
        let parsed = parse_deprecation_attrs(&attrs, Span::call_site()).unwrap().unwrap();
        assert_eq!(parsed.removed_in, DEPRECATION_REMOVED_IN_UNSPECIFIED);
    }

    /// Bare `#[deprecated]` (no args) yields empty-string defaults per the
    /// ticket's "Tests to add" item 3 variant.
    #[test]
    fn parse_deprecation_bare_emits_empty_since_and_message() {
        let attrs: Vec<syn::Attribute> = vec![parse_quote! { #[deprecated] }];
        let parsed = parse_deprecation_attrs(&attrs, Span::call_site()).unwrap().unwrap();
        assert_eq!(parsed.since, "");
        assert_eq!(parsed.message, "");
        assert_eq!(parsed.removed_in, DEPRECATION_REMOVED_IN_UNSPECIFIED);
    }

    /// Companion attr without a paired `#[deprecated]` is a compile error.
    #[test]
    fn parse_deprecation_companion_without_deprecated_errors() {
        let attrs: Vec<syn::Attribute> =
            vec![parse_quote! { #[plexus_macros::removed_in("0.6")] }];
        let err = parse_deprecation_attrs(&attrs, Span::call_site())
            .expect_err("removed_in alone must error");
        let msg = err.to_string();
        assert!(
            msg.contains("#[deprecated]"),
            "error must name #[deprecated] as required companion; got: {}",
            msg
        );
    }

    /// No deprecation attrs → `Ok(None)`.
    #[test]
    fn parse_deprecation_no_attrs_yields_none() {
        let attrs: Vec<syn::Attribute> = vec![parse_quote! { #[doc = "just a comment"] }];
        let parsed = parse_deprecation_attrs(&attrs, Span::call_site()).unwrap();
        assert!(parsed.is_none());
    }

    /// AUTHZ-MACRO-PARAM-DOC-1: `///` doc comment on a parameter is picked up
    /// as the param description when no explicit `params(name = "...")` entry
    /// exists on the method attribute.
    #[test]
    fn param_doc_comment_falls_back_when_params_attr_omits_entry() {
        let method: ImplItemFn = parse_quote! {
            async fn echo(
                &self,
                /// The message to echo back
                message: String,
            ) -> impl Stream<Item = MyEvent> + Send + 'static {
                stream! { yield MyEvent::Echo { message } }
            }
        };
        let info = MethodInfo::from_fn(&method, None).expect("parses cleanly");
        let param = info.params.iter().find(|p| p.name == "message").unwrap();
        assert_eq!(
            param.description.as_deref(),
            Some("The message to echo back"),
            "param doc comment should fall back into the description when no params(...) entry exists"
        );
    }

    /// AUTHZ-MACRO-PARAM-DOC-1: explicit `params(name = "...")` on the method
    /// attribute wins over a `///` doc comment on the parameter.
    #[test]
    fn explicit_params_attr_wins_over_param_doc_comment() {
        let method: ImplItemFn = parse_quote! {
            async fn echo(
                &self,
                /// IGNORED — explicit attr wins
                message: String,
            ) -> impl Stream<Item = MyEvent> + Send + 'static {
                stream! { yield MyEvent::Echo { message } }
            }
        };
        let attrs = HubMethodAttrs {
            name: None,
            description: None,
            param_docs: {
                let mut m = HashMap::new();
                m.insert("message".to_string(), "explicit description".to_string());
                m
            },
            bidirectional: BidirType::None,
            http_method: None,
            request_override: None,
            public: false,
            scopes: vec![],
        };
        let info = MethodInfo::from_fn(&method, Some(&attrs)).expect("parses cleanly");
        let param = info.params.iter().find(|p| p.name == "message").unwrap();
        assert_eq!(
            param.description.as_deref(),
            Some("explicit description"),
            "explicit params(...) entry must win over /// doc comment on the param"
        );
    }

    /// AUTHZ-MACRO-PARAM-DOC-1: parameter with neither explicit attr nor doc
    /// comment yields `description: None`.
    #[test]
    fn param_with_no_description_yields_none() {
        let method: ImplItemFn = parse_quote! {
            async fn echo(
                &self,
                message: String,
            ) -> impl Stream<Item = MyEvent> + Send + 'static {
                stream! { yield MyEvent::Echo { message } }
            }
        };
        let info = MethodInfo::from_fn(&method, None).expect("parses cleanly");
        let param = info.params.iter().find(|p| p.name == "message").unwrap();
        assert!(
            param.description.is_none(),
            "no description anywhere should yield None, got: {:?}",
            param.description
        );
    }

    /// R-1 (AUTHZ-CORE-2 AC #3): the compile-time scope-grammar check
    /// mirrors AUTHZ-CORE-4's `Scope::try_new` grammar.
    #[test]
    fn scope_literal_grammar_accepts_and_rejects() {
        // Accepted forms.
        for ok in [
            "facet.write",
            "cone.send_message",
            "a",
            "a.b.c",
            "a1.b_2",
            "cone.*",
            "a.b.*",
            "*",
        ] {
            assert!(
                validate_scope_literal(ok).is_ok(),
                "`{ok}` should satisfy the scope grammar"
            );
        }
        // Rejected forms.
        for bad in [
            "",
            "INVALID-CHARS",
            "Facet.write",
            "facet..write",
            ".facet",
            "facet.",
            "facet write",
            "1facet.write",
            "*.facet",      // wildcard must be trailing
            "facet.*.read", // wildcard must be trailing
            "**",
            "facet.wr*te",
        ] {
            assert!(
                validate_scope_literal(bad).is_err(),
                "`{bad}` should violate the scope grammar"
            );
        }
    }

    /// R-1: `scope = "..."` parses into `HubMethodAttrs::scopes` in
    /// declaration order; `public` + `scope` is rejected at parse time.
    #[test]
    fn method_attrs_parse_scopes_and_reject_public_scope_conflict() {
        let attrs: HubMethodAttrs =
            syn::parse_str("scope = \"x.y\", scope = \"x.z\"").expect("parses cleanly");
        assert_eq!(attrs.scopes, vec!["x.y".to_string(), "x.z".to_string()]);
        assert!(!attrs.public);

        let err = match syn::parse_str::<HubMethodAttrs>("public, scope = \"x.y\"") {
            Ok(_) => panic!("public + scope must be a parse error"),
            Err(e) => e,
        };
        assert!(
            err.to_string().contains("a public method has no required scope"),
            "unexpected error: {err}"
        );

        let err = match syn::parse_str::<HubMethodAttrs>("scope = \"BAD CHARS\"") {
            Ok(_) => panic!("invalid literal must be a parse error"),
            Err(e) => e,
        };
        assert!(
            err.to_string().contains("invalid scope literal"),
            "unexpected error: {err}"
        );
    }

    // ------------------------------------------------------------------
    // RETURNS-8: the signature forms and the implicit namespace
    // ------------------------------------------------------------------

    fn shape(sig: syn::Signature) -> syn::Result<ReturnShape> {
        classify_return(&sig)
    }

    fn tokens(ty: &Type) -> String {
        quote::quote!(#ty).to_string()
    }

    /// The six forms, each with its cardinality, `T`, `E` and `P`.
    #[test]
    fn the_six_forms_are_read_from_the_signature() {
        let cases: Vec<(syn::Signature, Cardinality, &str, Option<&str>, Option<&str>)> = vec![
            (parse_quote! { async fn a(&self) -> Pong }, Cardinality::One, "Pong", None, None),
            (
                parse_quote! { async fn b(&self) -> Result<Thing, LookupError> },
                Cardinality::One,
                "Thing",
                Some("LookupError"),
                None,
            ),
            (
                parse_quote! { async fn c(&self) -> impl Stream<Item = Tick> + Send + 'static },
                Cardinality::Many,
                "Tick",
                None,
                None,
            ),
            (
                parse_quote! { async fn d(&self) -> impl Send + Stream<Item = std::result::Result<Row, ScanError>> },
                Cardinality::Many,
                "Row",
                Some("ScanError"),
                None,
            ),
            (
                parse_quote! { async fn e(&self) -> impl Stream<Item = Result<Progress<Loaded, LoadStep>, LoadError>> },
                Cardinality::Many,
                "Loaded",
                Some("LoadError"),
                Some("LoadStep"),
            ),
            (
                parse_quote! { async fn f(&self) -> impl Stream<Item = plexus_client::Progress<Warmed>> },
                Cardinality::Many,
                "Warmed",
                None,
                Some("Note"),
            ),
        ];
        let vocabulary: syn::Path = parse_quote!(v);
        for (sig, cardinality, ok, err, progress) in cases {
            let name = sig.ident.to_string();
            let s = shape(sig).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(s.cardinality, cardinality, "{name}");
            assert_eq!(tokens(&s.ok_ty), ok, "{name}");
            assert_eq!(s.err_ty.as_ref().map(tokens).as_deref(), err, "{name}");
            let p = s.progress_ty.map(|p| match p {
                ProgressTy::Note => "Note".to_string(),
                declared => tokens(&declared.to_type(&vocabulary)),
            });
            assert_eq!(p.as_deref(), progress, "{name}");
        }
    }

    /// Pin 14: recognition is by name and arity — `anyhow::Result<T>` (one
    /// argument) is a plain `T`.
    #[test]
    fn a_one_argument_result_alias_is_a_plain_item() {
        let s = shape(parse_quote! { async fn a(&self) -> anyhow::Result<Pong> }).unwrap();
        assert_eq!(s.cardinality, Cardinality::One);
        assert_eq!(tokens(&s.ok_ty), "anyhow :: Result < Pong >");
        assert!(s.err_ty.is_none());
    }

    #[test]
    fn other_return_types_name_the_six_forms() {
        let bad: Vec<syn::Signature> = vec![
            parse_quote! { async fn a(&self) -> BoxStream<'static, Tick> },
            parse_quote! { async fn a(&self) -> Pin<Box<dyn Stream<Item = Tick> + Send>> },
            parse_quote! { async fn a(&self) -> PlexusStream },
            parse_quote! { async fn a(&self) -> Result<PlexusStream, PlexusError> },
            parse_quote! { async fn a(&self) -> impl Future<Output = Tick> },
            parse_quote! { async fn a(&self) -> Box<dyn Stream<Item = Tick>> },
            parse_quote! { async fn a(&self) },
        ];
        for sig in bad {
            let msg = match shape(sig) {
                Ok(_) => panic!("must be rejected"),
                Err(e) => e.to_string(),
            };
            assert!(msg.contains("six signature forms"), "{msg}");
            assert!(msg.contains("-> impl Stream<Item = Result<Progress<T, P>, E>>"), "{msg}");
        }
    }

    #[test]
    fn progress_needs_a_stream() {
        for sig in [
            parse_quote! { async fn a(&self) -> Progress<Tick> },
            parse_quote! { async fn a(&self) -> Result<Progress<Tick, Step>, E> },
        ] {
            let sig: syn::Signature = sig;
            let msg = match shape(sig) {
                Ok(_) => panic!("must be rejected"),
                Err(e) => e.to_string(),
            };
            assert!(msg.contains("cannot report progress"), "{msg}");
        }
    }

    #[test]
    fn streaming_and_returns_are_removed() {
        let err = match syn::parse_str::<HubMethodAttrs>("streaming") {
            Ok(_) => panic!("`streaming` must be rejected"),
            Err(e) => e.to_string(),
        };
        assert!(err.contains("cardinality comes from the return type — remove `streaming`"), "{err}");
        let err = match syn::parse_str::<HubMethodAttrs>("returns(Ok, Error)") {
            Ok(_) => panic!("`returns(..)` must be rejected"),
            Err(e) => e.to_string(),
        };
        assert!(err.contains("give each method its own return type"), "{err}");
    }

    #[test]
    fn the_implicit_namespace() {
        for (name, ns) in [
            ("Tickets", "tickets"),
            ("HyperforgeHub", "hyperforge"),
            ("InfoActivation", "info"),
            ("ClaudeCode", "claude_code"),
            ("Conformance", "conformance"),
            ("DynamicHub", "dynamic"),
            ("HTTPServer", "http_server"),
            ("V2Api", "v2_api"),
            ("Hub", "hub"),
            ("Activation", "activation"),
            ("echo", "echo"),
        ] {
            assert_eq!(implicit_namespace(name), ns, "{name}");
        }
    }
}
