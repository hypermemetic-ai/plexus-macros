//! # plexus-macros
//!
//! Procedural macros for **Plexus RPC** activations. Your Rust function
//! signature IS the schema — the macros extract method names, parameter
//! types, return types, doc comments, deprecation metadata, child routing,
//! and typed request extraction from the source.
//!
//! See the crate-level [`README`](https://github.com/hypermemetic/plexus-macros/blob/main/README.md)
//! for a comprehensive example covering every option, a deep-dive on
//! request forwarding (`PlexusRequest` + `request = ...`), and a
//! per-macro reference.
//!
//! # Quick example
//!
//! ```ignore
//! use futures::Stream;
//! use async_stream::stream;
//! use plexus_macros::PlexusRequest;
//!
//! #[derive(PlexusRequest)]
//! pub struct BashRequest {
//!     #[from_cookie("access_token")]
//!     auth_token: String,
//! }
//!
//! pub struct Bash;
//!
//! #[plexus_macros::activation(
//!     namespace = "bash",
//!     version = "1.0.0",
//!     description = "Execute bash commands and stream output",
//!     request = BashRequest,
//! )]
//! impl Bash {
//!     /// Execute a bash command and stream output.
//!     #[plexus_macros::method(streaming)]
//!     async fn execute(
//!         &self,
//!         command: String,
//!         #[activation_param] auth_token: String,
//!     ) -> impl Stream<Item = String> + Send + 'static {
//!         let _ = (command, auth_token);
//!         stream! { yield "ok".into(); }
//!     }
//! }
//! ```
//!
//! # Macros at a glance
//!
//! | Macro | Purpose |
//! |---|---|
//! | `#[plexus_macros::activation]` | Turn an `impl` block into a Plexus RPC activation. |
//! | `#[plexus_macros::method]` | Mark a method as an RPC endpoint. |
//! | `#[plexus_macros::child]` | Register a child activation for `ChildRouter` dispatch. |
//! | `#[plexus_macros::removed_in]` | Companion to `#[deprecated]` carrying the removal version. |
//! | `#[derive(PlexusRequest)]` | Typed extraction from `RawRequestContext`. |
//! | `#[derive(HandleEnum)]` | Type-safe handle enums for an activation. |
//!
//! Deprecated surfaces kept through 0.5.x (removed in 0.6): `#[hub_methods]`,
//! `#[hub_method]`, `#[derive(StreamEvent)]`, the `hub` and `children = [...]`
//! flags on `#[activation]`. See the migration section of the README.
//!
//! ## Crate version (UMB-2)
//!
//! This crate is `proc-macro = true`, which means it can only export macro
//! attributes — `pub const VERSION` etc. are forbidden by the compiler. The
//! `plexus-rpc` umbrella reads `plexus-macros`'s version via a build-time
//! environment variable populated from `Cargo.toml`'s dependency pin
//! (see `plexus-rpc/build.rs`).

mod codegen;
mod credential;
mod handle_enum;
mod ir_parse;
mod parse;
mod request;
mod stream_event;


use codegen::generate_all;
use parse::HubMethodsAttrs;
use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::{
    parse::{Parse, ParseStream},
    parse_macro_input, punctuated::Punctuated, Expr, ExprLit, FnArg, ItemFn, ItemImpl, Lit, Meta,
    MetaNameValue, Pat, ReturnType, Token, Type,
};

/// Parsed attributes for hub_method (standalone version)
struct HubMethodAttrs {
    name: Option<String>,
    /// Explicit description from `description = "..."`. When `Some`, this wins over
    /// `///` doc comments. When `None`, doc comments are used as the default.
    description: Option<String>,
    /// Base crate path for imports (default: "crate")
    crate_path: String,
}

impl Parse for HubMethodAttrs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut name = None;
        let mut description: Option<String> = None;
        let mut crate_path = "crate".to_string();

        if !input.is_empty() {
            let metas = Punctuated::<Meta, Token![,]>::parse_terminated(input)?;

            for meta in metas {
                if let Meta::NameValue(MetaNameValue { path, value, .. }) = meta {
                    if path.is_ident("name") {
                        if let Expr::Lit(ExprLit {
                            lit: Lit::Str(s), ..
                        }) = value
                        {
                            name = Some(s.value());
                        }
                    } else if path.is_ident("description") {
                        if let Expr::Lit(ExprLit {
                            lit: Lit::Str(s), ..
                        }) = value
                        {
                            description = Some(s.value());
                        }
                    } else if path.is_ident("crate_path") {
                        if let Expr::Lit(ExprLit {
                            lit: Lit::Str(s), ..
                        }) = value
                        {
                            crate_path = s.value();
                        }
                    }
                }
            }
        }

        Ok(HubMethodAttrs { name, description, crate_path })
    }
}

/// Attribute macro for individual methods within a `#[plexus::activation]` impl block.
///
/// # Example
///
/// ```ignore
/// #[plexus::activation(namespace = "bash")]
/// impl Bash {
///     /// Execute a bash command
///     #[plexus::method]
///     async fn execute(&self, command: String) -> impl Stream<Item = BashEvent> {
///         // ...
///     }
/// }
/// ```
///
/// # Authorization arguments (R-1 / AUTHZ-CORE-2)
///
/// - `scope = "..."` — explicit required scope, emitted into the generated
///   `MethodSchema.requires_credential`. Repeatable: `scope = "x.y",
///   scope = "x.z"` means the caller must satisfy **ALL** listed scopes
///   (conjunction — AND, not OR). Literals are validated at compile time
///   against the AUTHZ-CORE-4 scope grammar (dot-separated `[a-z][a-z0-9_]*`
///   segments, optional trailing `.*` segment wildcard, or bare `*`).
/// - `public` — the method is exempt from the default-deny gate; emitted as
///   `MethodSchema.public = true`. Mixing `public` with `scope = "..."` is a
///   compile error (a public method has no required scope to enforce).
/// - Neither — the implicit `<activation>.<method>` scope rule applies at
///   registry-build time (AUTHZ-CORE-4); the schema carries no
///   `requires_credential` for the implicit case.
#[proc_macro_attribute]
pub fn method(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as HubMethodAttrs);
    let input_fn = parse_macro_input!(item as ItemFn);

    match hub_method_impl(args, input_fn) {
        Ok(tokens) => tokens.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// Deprecated: use `plexus::method` instead.
#[deprecated(since = "0.5.0", note = "Use `plexus::method` instead")]
#[proc_macro_attribute]
pub fn hub_method(attr: TokenStream, item: TokenStream) -> TokenStream {
    method(attr, item)
}

/// Attribute macro for individual **child** methods within a
/// `#[plexus_macros::activation]` impl block.
///
/// Two method shapes are accepted:
///
/// | Shape | Role | Example |
/// |---|---|---|
/// | `fn NAME(&self) -> Child` (no args) | Static child — routing name is the method name | `fn mercury(&self) -> Mercury` |
/// | `fn NAME(&self, name: &str) -> Option<Child>` (sync or async) | Dynamic fallback dispatcher | `fn planet(&self, name: &str) -> Option<Planet>` |
///
/// The enclosing `#[plexus_macros::activation]` macro collects all `#[child]`
/// methods and generates a `ChildRouter` impl whose `get_child(name)`
/// dispatches over them. `#[child]` methods are **not** exposed as Plexus RPC
/// methods — they contribute only to `ChildRouter` routing.
///
/// # Example
///
/// ```ignore
/// #[plexus_macros::activation(namespace = "solar")]
/// impl Solar {
///     #[plexus_macros::child]
///     fn mercury(&self) -> Mercury { self.mercury.clone() }
///
///     #[plexus_macros::child]
///     async fn planet(&self, name: &str) -> Option<Planet> {
///         self.lookup_planet(name).await
///     }
/// }
/// ```
///
/// Used standalone (outside an `#[activation]` block) this attribute is a
/// no-op that returns the function unchanged.
#[proc_macro_attribute]
pub fn child(_attr: TokenStream, item: TokenStream) -> TokenStream {
    // Outside of #[plexus_macros::activation] this attribute does nothing;
    // the activation macro strips it and collects the method into the
    // generated ChildRouter impl.
    item
}

/// Companion attribute to Rust's built-in `#[deprecated]` for carrying a
/// `removed_in = "VERSION"` hint that rustc doesn't recognize.
///
/// Rust's `#[deprecated(since = "X", note = "Y")]` supports only `since` and
/// `note`. This companion attribute supplies the removal version that the
/// `plexus-macros` codegen folds into the method's `DeprecationInfo.removed_in`
/// field on the emitted `MethodSchema`.
///
/// # Example
///
/// ```ignore
/// #[deprecated(since = "0.5", note = "use `new_method` instead")]
/// #[plexus_macros::removed_in("0.6")]
/// #[plexus_macros::method]
/// async fn legacy(&self) -> impl Stream<Item = String> + Send + 'static {
///     // ...
/// }
/// ```
///
/// # Requirements
///
/// `#[plexus_macros::removed_in]` is only meaningful when paired with
/// `#[deprecated]`. The `#[plexus_macros::activation]` macro emits a compile
/// error when it encounters `#[removed_in]` on a method that doesn't also
/// carry `#[deprecated]`.
///
/// As a standalone attribute (outside an `#[activation]` block) this is a
/// no-op that returns the item unchanged — the activation macro parses and
/// strips it during codegen.
#[proc_macro_attribute]
pub fn removed_in(attr: TokenStream, item: TokenStream) -> TokenStream {
    // When `#[plexus_macros::removed_in("X")]` is placed OUTSIDE
    // `#[plexus_macros::activation(...)]` on the same impl block, the
    // `removed_in` proc macro runs BEFORE the activation macro (outer
    // attrs expand first), so a naive no-op implementation would consume
    // the attribute before the activation macro ever sees it.
    //
    // Workaround: emit a synthetic `#[doc(hidden)]` marker attribute on
    // the item's attr list that encodes the removed_in value. The
    // activation / method codegen scans for this marker as an equivalent
    // signal to the companion attribute and uses it to populate
    // `DeprecationInfo.removed_in`.
    //
    // Stable rustc treats `#[doc(hidden)]` as a regular doc attribute and
    // the sentinel payload rides along in the same literal string.
    let attr_str: String = match syn::parse::<syn::LitStr>(attr.clone()) {
        Ok(s) => s.value(),
        Err(_) => {
            // Not a bare string literal — just let the item pass through
            // and let the activation/method layer produce the proper error.
            return item;
        }
    };
    let marker = format!("__plexus_removed_in:{}", attr_str);
    let item_ts: proc_macro2::TokenStream = item.into();
    let marker_lit = proc_macro2::Literal::string(&marker);
    let out = quote::quote! {
        #[doc(hidden)]
        #[doc = #marker_lit]
        #item_ts
    };
    out.into()
}

fn hub_method_impl(args: HubMethodAttrs, input_fn: ItemFn) -> syn::Result<TokenStream2> {
    // Extract method name (from attr or function name)
    let method_name = args
        .name
        .unwrap_or_else(|| input_fn.sig.ident.to_string());

    // Resolve description: explicit `description = "..."` wins over `///` doc
    // comments; if neither is present, description is the empty string.
    let description = match args.description {
        Some(explicit) => explicit,
        None => extract_doc_comment(&input_fn),
    };

    // Extract input type from first parameter (if any)
    let input_type = extract_input_type(&input_fn)?;

    // Extract return type
    let return_type = extract_return_type(&input_fn)?;

    // Function name for the schema function
    let fn_name = &input_fn.sig.ident;
    let schema_fn_name = format_ident!("{}_schema", fn_name);

    // Parse crate path
    let crate_path: syn::Path = syn::parse_str(&args.crate_path)
        .map_err(|e| syn::Error::new_spanned(&input_fn.sig, format!("Invalid crate_path: {}", e)))?;

    // Generate the schema function
    let schema_fn = generate_schema_fn(
        &schema_fn_name,
        &method_name,
        &description,
        input_type.as_ref(),
        &return_type,
        &crate_path,
    );

    // Return the original function plus the schema function
    Ok(quote! {
        #input_fn

        #schema_fn
    })
}

fn extract_doc_comment(input_fn: &ItemFn) -> String {
    // Delegate to the shared helper so the standalone `#[method]` macro path
    // follows the same doc-comment extraction rule as the in-activation path:
    // lines joined with '\n', common leading whitespace stripped.
    parse::extract_doc_description(&input_fn.attrs).unwrap_or_default()
}

fn extract_input_type(input_fn: &ItemFn) -> syn::Result<Option<Type>> {
    // Skip self parameter, get first real parameter
    for arg in &input_fn.sig.inputs {
        match arg {
            FnArg::Receiver(_) => continue, // Skip &self
            FnArg::Typed(pat_type) => {
                // Skip context-like parameters
                if let Pat::Ident(ident) = &*pat_type.pat {
                    let name = ident.ident.to_string();
                    if name == "ctx" || name == "context" || name == "self_" {
                        continue;
                    }
                }
                return Ok(Some((*pat_type.ty).clone()));
            }
        }
    }

    Ok(None)
}

fn extract_return_type(input_fn: &ItemFn) -> syn::Result<Type> {
    match &input_fn.sig.output {
        ReturnType::Default => Err(syn::Error::new_spanned(
            &input_fn.sig,
            "hub_method requires a return type",
        )),
        ReturnType::Type(_, ty) => Ok((*ty.clone()).clone()),
    }
}

fn generate_schema_fn(
    fn_name: &syn::Ident,
    method_name: &str,
    description: &str,
    input_type: Option<&Type>,
    return_type: &Type,
    crate_path: &syn::Path,
) -> TokenStream2 {
    // Z2H-2: the standalone `#[method]` macro has no activation-level shim
    // module, so resolve the dep roots absolutely: through the plexus-rpc
    // umbrella's `__deps` re-exports when the consumer depends on it, or the
    // extern prelude (today's semantics) otherwise.
    let (serde_json_root, schemars_root) =
        match codegen::umbrella_crate(proc_macro2::Span::call_site()) {
            Some(umbrella) => (
                quote! { ::#umbrella::__deps::serde_json },
                quote! { ::#umbrella::__deps::schemars },
            ),
            None => (quote! { serde_json }, quote! { schemars }),
        };

    let input_schema = if let Some(input_ty) = input_type {
        quote! {
            Some(#serde_json_root::to_value(#schemars_root::schema_for!(#input_ty)).unwrap())
        }
    } else {
        quote! { None }
    };

    let _ = return_type; // Will be used for protocol schema in future
    let _ = crate_path;

    quote! {
        /// Generated schema function for this hub method
        #[allow(dead_code)]
        pub fn #fn_name() -> #serde_json_root::Value {
            #serde_json_root::json!({
                "name": #method_name,
                "description": #description,
                "input": #input_schema,
            })
        }
    }
}

/// Attribute macro for impl blocks defining a Plexus activation.
///
/// Generates:
/// - Method enum for schema extraction
/// - Activation trait implementation
/// - RPC server trait and implementation
///
/// # Attributes
///
/// - `namespace = "..."` (required) - The activation namespace
/// - `version = "..."` (optional, default: "1.0.0") - Version string
/// - `description = "..."` (optional) - Activation description
/// - `crate_path = "..."` (optional, default: "crate") - Path to substrate crate
///
/// # Example
///
/// ```ignore
/// #[plexus::activation(namespace = "bash", version = "1.0.0", description = "Execute bash commands")]
/// impl Bash {
///     /// Execute a bash command and stream output
///     #[plexus::method]
///     async fn execute(&self, command: String) -> impl Stream<Item = BashEvent> + Send + 'static {
///         self.executor.execute(&command).await
///     }
/// }
/// ```
#[proc_macro_attribute]
pub fn activation(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as HubMethodsAttrs);
    let input_impl = parse_macro_input!(item as ItemImpl);

    match generate_all(args, input_impl) {
        Ok(tokens) => tokens.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// Deprecated: use `plexus::activation` instead.
#[deprecated(since = "0.5.0", note = "Use `plexus::activation` instead")]
#[proc_macro_attribute]
pub fn hub_methods(attr: TokenStream, item: TokenStream) -> TokenStream {
    activation(attr, item)
}

/// Derive macro for declaring credential-bearing fields on a return-type
/// struct or enum (per `AUTHZ-CRED-MACRO-1`).
///
/// Apply `#[derive(Credentials)]` to a struct or enum. Within the item,
/// annotate each credential-bearing field with the helper attribute
/// `#[credential(kind = "...", attach_as = "...", ...)]` carrying the
/// per-field metadata. The field's type must be `Credential<T>` (the sealed
/// wrapper from `plexus_auth_core`); any other type produces a compile
/// error citing the wrapper requirement.
///
/// **Why a derive macro and not a freestanding attribute macro?** Stable Rust
/// proc-macros cannot intercept attribute macros applied directly to struct
/// fields; only derive-helper attributes can appear there. See
/// `plans/AUTHZ/AUTHZ-CRED-MACRO-1-RUN-NOTES.md` for the rationale.
///
/// # Example
///
/// ```ignore
/// use plexus_auth_core::{
///     AttachmentSite, Credential, CredentialKind, CredentialScheme,
///     HeaderName, MethodPath, Scope,
/// };
///
/// // `CredentialFieldMarker` lands in plexus-auth-core in a follow-up; for
/// // now the macro emits code that references it by name and consumers
/// // provide the definition (see run-notes).
///
/// #[derive(plexus_macros::Credentials, serde::Serialize)]
/// pub struct LoginEvent {
///     pub user_id: String,
///
///     #[credential(
///         kind = "bearer",
///         attach_as = "header:authorization",
///         scheme = "Bearer ",
///     )]
///     pub session: Credential<String>,
/// }
/// ```
///
/// # Clauses
///
/// | Clause | Required? | Notes |
/// |---|---|---|
/// | `kind = "<name>"` | required | Closed enum: `"bearer"`, `"cookie"`, `"oauth_access"`, `"oauth_refresh"`, `"oidc_id"`, `"aws_sts"`, `"macaroon"`. The escape valve `kind(other = "<name>")` carries a backend-specific kind. |
/// | `attach_as = "<spec>"` | required | One of `"header:<name>"`, `"cookie:<name>"`, `"first_frame:<setup-method>:<param>"`, `"in_rpc_param:<name>"`. |
/// | `scheme = "<prefix>"` | optional | E.g. `"Bearer "` (trailing space significant). |
/// | `scopes = ["a.b", ...]` | optional, default empty | List of scope identifiers. |
/// | `refresh_via = "<method-path>"` | optional | E.g. `"auth.refresh"`. |
/// | `revoke_via = "<method-path>"` | optional | E.g. `"auth.logout"`. |
/// | `sensitive` | optional flag | Accepted for clarity; credentials are always sensitive. |
///
/// # Emitted artifacts
///
/// The macro emits two free functions in the same module:
///
/// - `fn __plexus_credential_marker_for_<TypeIdent>() -> Vec<CredentialFieldMarker>`
/// - `fn __plexus_sensitive_fields_for_<TypeIdent>() -> Vec<(Option<&'static str>, &'static str)>`
///
/// The consumer (per `AUTHZ-CRED-CORE-3`) calls the first function during
/// schema build; the second mirrors the implicit `#[sensitive]` rule for the
/// redaction pipeline (per `AUTHZ-PRIVACY-2`).
///
/// The emitted code requires `CredentialFieldMarker`, `CredentialKind`,
/// `AttachmentSite`, `CredentialScheme`, `Scope`, `MethodPath`, `HeaderName`,
/// `CookieName`, `ParamName`, and `CredentialKindName` to be in scope at the
/// emission site. Importing `plexus_auth_core::*` (or the equivalent named
/// imports) at the top of the consuming module satisfies all but
/// `CredentialFieldMarker`, which lands in `plexus_auth_core` in a follow-up.
#[proc_macro_derive(Credentials, attributes(credential))]
pub fn credentials_derive(input: TokenStream) -> TokenStream {
    credential::expand_credentials_derive(input)
}

/// **DEPRECATED**: This derive macro is no longer needed.
///
/// With the caller-wraps streaming architecture, event types no longer need to
/// implement `ActivationStreamItem`. Just use plain domain types with standard
/// derives:
///
/// ```ignore
/// #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
/// #[serde(tag = "event", rename_all = "snake_case")]
/// pub enum MyEvent {
///     Data { value: String },
///     Complete { result: i32 },
/// }
/// ```
///
/// The wrapping happens at the call site via `wrap_stream()`.
#[deprecated(
    since = "0.2.0",
    note = "No longer needed - use plain domain types with Serialize/Deserialize"
)]
#[proc_macro_derive(StreamEvent, attributes(stream_event, terminal))]
pub fn stream_event_derive(input: TokenStream) -> TokenStream {
    stream_event::derive(input)
}

/// Derive macro for type-safe handle creation and parsing.
///
/// Generates:
/// - `to_handle(&self) -> Handle` - converts enum variant to Handle
/// - `impl TryFrom<&Handle>` - parses Handle back to enum variant
/// - `impl From<EnumName> for Handle` - convenience conversion
///
/// # Attributes
///
/// Enum-level (required):
/// - `plugin_id = "CONSTANT_NAME"` - Name of the constant holding the plugin UUID
/// - `version = "1.0.0"` - Semantic version for handles
/// - `crate_path = "..."` (optional, default: "plexus_core") - Path to plexus_core crate
/// - `plugin_id_type = "Type<Args>"` (optional) - Concrete type whose associated
///   constant `plugin_id` names. Use this when the owning activation is generic
///   (e.g. `Cone<P: HubContext = NoParent>`) to pin the instantiation and avoid
///   E0283 "cannot infer type" errors. Pairs with `plugin_id` — e.g.
///   `plugin_id = "Cone::PLUGIN_ID", plugin_id_type = "Cone<NoParent>"` emits
///   `<Cone<NoParent>>::PLUGIN_ID`.
///
/// Variant-level:
/// - `method = "..."` (required) - The handle.method value
/// - `table = "..."` (optional) - SQLite table name (for future resolution)
/// - `key = "..."` (optional) - Primary key column (for future resolution)
///
/// # Example
///
/// ```ignore
/// use hub_macro::HandleEnum;
/// use uuid::Uuid;
///
/// pub const MY_PLUGIN_ID: Uuid = uuid::uuid!("550e8400-e29b-41d4-a716-446655440000");
///
/// #[derive(HandleEnum)]
/// #[handle(plugin_id = "MY_PLUGIN_ID", version = "1.0.0")]
/// pub enum MyPluginHandle {
///     #[handle(method = "event", table = "events", key = "id")]
///     Event { event_id: String },
///
///     #[handle(method = "message")]
///     Message { message_id: String, role: String },
/// }
///
/// // Usage:
/// let handle_enum = MyPluginHandle::Event { event_id: "evt-123".into() };
/// let handle: Handle = handle_enum.to_handle();
/// // handle.method == "event"
/// // handle.meta == ["evt-123"]
///
/// // Parsing back:
/// let parsed = MyPluginHandle::try_from(&handle)?;
/// ```
#[proc_macro_derive(HandleEnum, attributes(handle))]
pub fn handle_enum_derive(input: TokenStream) -> TokenStream {
    handle_enum::derive(input)
}

/// Derive macro for typed HTTP/WebSocket request extraction.
///
/// Generates `impl PlexusRequest` (extraction) and `impl schemars::JsonSchema`
/// (with `x-plexus-source` extensions) for the annotated struct.
///
/// # Field annotations
///
/// - `#[from_cookie("name")]` — extract from named cookie
/// - `#[from_header("name")]` — extract from named HTTP header
/// - `#[from_query("name")]` — extract from named URI query parameter
/// - `#[from_peer]` — copy `ctx.peer` (peer socket address)
/// - `#[from_auth_context]` — copy `ctx.auth` (auth context)
/// - (no annotation) — `Default::default()`
///
/// # Example
///
/// ```ignore
/// use plexus_macros::PlexusRequest;
///
/// #[derive(PlexusRequest)]
/// struct MyRequest {
///     #[from_cookie("access_token")]
///     auth_token: String,
///
///     #[from_header("origin")]
///     origin: Option<String>,
///
///     #[from_peer]
///     peer_addr: Option<std::net::SocketAddr>,
/// }
/// ```
#[proc_macro_derive(
    PlexusRequest,
    attributes(from_cookie, from_header, from_query, from_peer, from_auth_context)
)]
pub fn plexus_request_derive(input: TokenStream) -> TokenStream {
    request::derive(input)
}

/// No-op `JsonSchema` derive — used when plexus-macros is aliased as `schemars`
/// in dev-dependencies to prevent duplicate `impl JsonSchema` conflicts with
/// `#[derive(PlexusRequest)]` (which generates its own `impl JsonSchema`).
///
/// When plexus-macros is aliased as schemars in dev-deps:
/// ```toml
/// [dev-dependencies]
/// schemars = { path = "../plexus-macros", package = "plexus-macros", features = ["schemars-compat"] }
/// ```
/// then `#[derive(schemars::JsonSchema)]` invokes THIS derive instead of the real schemars derive,
/// producing no output, so only `PlexusRequest`'s `impl JsonSchema` exists.
#[cfg(feature = "schemars-compat")]
#[proc_macro_derive(JsonSchema, attributes(schemars, serde))]
pub fn json_schema_noop_derive(_input: TokenStream) -> TokenStream {
    // Intentionally produces no output.
    // PlexusRequest already generates impl JsonSchema with x-plexus-source extensions.
    TokenStream::new()
}

// ===========================================================================
// PLX-79 (M1·B) — the vNext parser, additive
// ===========================================================================
//
// These two entry points are the ONLY registration the vNext parser adds. They
// do not touch `#[activation]`, `#[method]`, or anything those emit: the
// existing path is byte-for-byte unchanged, and build F performs the
// switchover by calling `ir_parse::{method_ir_expr, activation_ir_expr}`
// directly from `#[activation]` instead of through these macros.

/// Build a `plexus_core::ir::MethodIr` from one `#[method(...)]`-annotated
/// signature (PLX-79 / M1·B).
///
/// Expands to an expression of type
/// `Result<plexus_core::ir::MethodIr, plexus_core::ir::SchemaRefError>`. It is
/// a `Result` because `SchemaRef::new` is fallible by design (PLX-75): a type
/// whose JSON Schema carries no information cannot be silently degraded, and a
/// generated path must not `.expect()` it away.
///
/// ```ignore
/// let ir = plexus_macros::method_ir! {
///     namespace = "claudecode",
///     /// Send a prompt.
///     #[method(http_method = "POST", ext(acp_role = "prompt"))]
///     async fn chat(&self, prompt: String, client: Client<(Permission, FsRead)>)
///         -> impl Stream<Item = Update>;
/// }.unwrap();
///
/// assert_eq!(ir.dotted_id, "claudecode.chat");
/// assert!(ir.is_streaming());
/// // `client` is not a parameter — it is the callback declaration.
/// assert_eq!(ir.params.len(), 1);
/// assert_eq!(ir.callbacks, Client::<(Permission, FsRead)>::callbacks());
/// ```
///
/// Unknown keys inside `#[method(...)]` are **errors** naming the key; use
/// `ext(...)` for open metadata.
#[proc_macro]
pub fn method_ir(input: TokenStream) -> TokenStream {
    let parsed = parse_macro_input!(input as ir_parse::MethodIrInput);
    match ir_parse::method_ir_expr(parsed) {
        Ok(tokens) => tokens.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// Build a `plexus_core::ir::ActivationIr` from an `#[activation]`-shaped impl
/// block — every `#[method]` plus a child edge per `#[child]` (PLX-79 / M1·B).
///
/// Expands to an expression of type
/// `Result<plexus_core::ir::ActivationIr, plexus_core::ir::SchemaRefError>`
/// with every Merkle hash already computed.
///
/// ```ignore
/// let ir = plexus_macros::activation_ir! {
///     namespace = "claudecode",
///     version = "1.0.0",
///     backend_name = "substrate",
///     impl ClaudeCode {
///         #[method]
///         async fn list(&self) -> Result<Vec<String>, Error>;
///
///         #[child(list = "list")]
///         fn session(&self) -> Session;
///     }
/// }.unwrap();
/// ```
///
/// A `#[child]` edge embeds the child's own document, obtained from the child
/// type's `__plexus_activation_ir()` associated function.
#[proc_macro]
pub fn activation_ir(input: TokenStream) -> TokenStream {
    let parsed = parse_macro_input!(input as ir_parse::ActivationIrInput);
    match ir_parse::activation_ir_expr(parsed) {
        Ok(tokens) => tokens.into(),
        Err(e) => e.to_compile_error().into(),
    }
}
