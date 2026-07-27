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
//! | `#[plexus_macros::acp_agent]` | Turn an `impl` block into an ACP v1 agent, with `AgentCapabilities` derived from what it implements. |
//! | `#[plexus_macros::method]` | Mark a method as an RPC endpoint. |
//! | `#[plexus_macros::child]` | Register a child activation for `ChildRouter` dispatch. |
//! | `#[plexus_macros::removed_in]` | Companion to `#[deprecated]` carrying the removal version. |
//! | `#[derive(PlexusRequest)]` | Typed extraction from `RawRequestContext`. |
//! | `#[derive(HandleEnum)]` | Type-safe handle enums for an activation. |
//!
//! Deprecated surfaces kept through 0.5.x: `#[hub_methods]`, `#[hub_method]`,
//! the `hub` and `children = [...]` flags on `#[activation]`. See the migration
//! section of the README.
//!
//! PLX-92 retired `#[derive(Credentials)]` and `#[derive(StreamEvent)]`, which
//! the fresh consumer census found had zero call sites in plexus-core,
//! plexus-transport, plexus-substrate, plexus-idp, plexus-codegen, hyperforge
//! or connectome-hs. The `#[hub_methods]` / `#[hub_method]` aliases survived
//! that pass only because `plexus-core/src/lib.rs` still `pub use`s them; they
//! go when that re-export does.
//!
//! ## Crate version (UMB-2)
//!
//! This crate is `proc-macro = true`, which means it can only export macro
//! attributes — `pub const VERSION` etc. are forbidden by the compiler. The
//! `plexus-rpc` umbrella reads `plexus-macros`'s version via a build-time
//! environment variable populated from `Cargo.toml`'s dependency pin
//! (see `plexus-rpc/build.rs`).

mod acp_agent;
mod codegen;
mod handle_enum;
mod ir_parse;
mod parse;
mod request;

/// PLX-93 / M1-T3 — the sealed-seam assertion, run against the real
/// `#[activation]` expansion. Test-only.
#[cfg(test)]
mod plx93_seam_tests;


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
/// - `backend_name = "..."` (optional) - RFC 002 §3.3 **root fact**: the
///   document's backend identity. Declaring it is what lets a client skip the
///   separate identity probe. Same spelling as `activation_ir!`.
/// - `respond_method = "..."` (optional) - RFC 002 §3.3/§7.6 **root fact**: the
///   name of the method a client calls to answer a callback this document
///   declares. §3.3 makes it MUST as soon as any method declares a callback,
///   because §7.6 requires a consumer to be able to tell *before invoking*
///   whether it can serve one. Same spelling as `activation_ir!`.
///
/// - `auth = "required" | "optional" | "mixed" | "none"` (optional) — RED-6
///   declared authorization posture, stamped into every generated
///   `MethodSchema.auth_posture`. **The key is `auth`.** It is not
///   `auth_posture`; that is the name of the parsed field, and writing it as a
///   key is a compile error (PLX-147).
/// - `resolve_handle`, `hub` — bare flags, no value.
/// - `children(a, b)` / `children = [a, b]` — deprecated child field list; use
///   `#[plexus_macros::child]` accessor methods instead.
/// - `required(a, b)` / `required = [a, b]` — REQ-10 activation-level lock list.
/// - `request = SomeType` — activation-level request type.
///
/// Both root facts belong to the *document*, not the node: `recompute_hashes`
/// strips them from every embedded child (§3.3), and §4.6 keeps them out of the
/// activation preimage, so declaring them changes no activation or method hash.
/// Neither may be the empty string (§3.5).
///
/// # Unknown arguments are compile errors (PLX-147)
///
/// An argument that is not on the list above is an error naming the key and
/// listing the accepted set — the same posture `#[method(...)]` has had since
/// PLX-93. So is a recognized key written in a form the parser does not read
/// (`auth = mixed` without quotes, `hub = true`).
///
/// This was not free strictness. The parser previously matched the keys it knew
/// and dropped the rest in silence, so `#[activation(bakcend_name = "oops")]`
/// compiled clean and declared nothing. A fleet census found five live
/// casualties of exactly that: `plexus-idp`'s `identity` hub and four
/// `plexus-trak` hubs all wrote `auth_posture = "mixed"`, so none of them had
/// any declared authorization posture while every one of them read as though it
/// did. The spelling came from this crate's own RED-2 diagnostic, which
/// advertised the wrong key.
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

/// PLX-136 (ACP·B) — impose `plexus_acp::v1::Agent`, and derive the
/// capabilities from what you implemented.
///
/// Put it on an **inherent** `impl` block whose items are ACP v1 methods. The
/// macro moves those bodies aside and emits
/// `impl plexus_acp::v1::Agent for T` in their place, so:
///
/// - a misspelled item is `E0407`, with the expected signature printed;
/// - a missing required item is `E0046`, with the whole surface enumerated;
/// - `initialize` is **generated**, and writing one yourself is a compile
///   error — it is the only thing that could return a hand-written
///   `AgentCapabilities`.
///
/// # Advertising and implementing are one act
///
/// `AgentCapabilities` is derived from the idents in the block, through
/// `plexus_acp::v1::derive_agent_capabilities`. There is no attribute key
/// that declares a capability and no method body that could return one, so
/// `loadSession: true` without a `load_session` impl is not a state this
/// macro can produce — and a `load_session` impl that is never advertised is
/// not one either.
///
/// PLX-135 established that this is three sites, not one:
///
/// | site | what it advertises | how it is discharged |
/// |---|---|---|
/// | `InitializeResponse.agentCapabilities` | seven optional items | derived; the generated `initialize` is the only writer |
/// | `NewSessionResponse.modes` / `.configOptions` | `set_session_mode`, `set_session_config_option` | checked both ways in the generated `new_session` wrapper |
/// | `InitializeResponse.authMethods` | `authenticate` | `Self::acp_auth_methods()` is called **iff** `authenticate` is implemented; an empty list is an error |
///
/// # Keys
///
/// `acp_path`, `name`, `version` — and nothing else. `name` and `version`
/// populate `agentInfo` (identity, not capability) and must be given
/// together. Every capability-shaped key is a compile error naming the impl
/// to write instead.
///
/// # Example
///
/// ```ignore
/// use plexus_acp::v1::schema::*;
/// use plexus_acp::v1::Result;
///
/// pub struct MyAgent;
///
/// #[plexus_macros::acp_agent(name = "my-agent", version = "0.1.0")]
/// impl MyAgent {
///     async fn new_session(&self, _r: NewSessionRequest) -> Result<NewSessionResponse> {
///         Ok(NewSessionResponse::new("s-1"))
///     }
///     async fn prompt(&self, _r: PromptRequest) -> Result<PromptResponse> {
///         Ok(PromptResponse::new(StopReason::EndTurn))
///     }
///     async fn cancel(&self, _n: CancelNotification) -> Result<()> {
///         Ok(())
///     }
///     // Implementing this — and only implementing it — advertises
///     // `agentCapabilities.loadSession`.
///     async fn load_session(&self, _r: LoadSessionRequest) -> Result<LoadSessionResponse> {
///         Ok(LoadSessionResponse::new())
///     }
/// }
/// ```
#[proc_macro_attribute]
pub fn acp_agent(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as acp_agent::AcpAgentAttrs);
    let input_impl = parse_macro_input!(item as ItemImpl);

    match acp_agent::generate(&args, input_impl) {
        Ok(tokens) => tokens.into(),
        Err(e) => e.to_compile_error().into(),
    }
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
