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
//! #[plexus_macros::activation(            // namespace "bash", implicitly
//!     version = "1.0.0",
//!     description = "Execute bash commands and stream output",
//!     request = BashRequest,
//! )]
//! impl Bash {
//!     /// Execute a bash command and stream output.
//!     #[plexus_macros::method]
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
//! | `#[plexus_macros::activation]` | Turn an `impl` block into a Plexus RPC activation — and, with `client`, emit its typed client. |
//! | `#[plexus_macros::method]` | Mark a method as an RPC endpoint. |
//! | `#[plexus_macros::child]` | Register a child activation for `ChildRouter` dispatch. |
//! | `#[plexus_macros::removed_in]` | Companion to `#[deprecated]` carrying the removal version. |
//! | `#[derive(PlexusRequest)]` | Typed extraction from `RawRequestContext`. |
//! | `#[derive(HandleEnum)]` | Type-safe handle enums for an activation. |
//! | `#[derive(plexus_macros::Deserialize)]` | An internally tagged event's `Deserialize`, its `type` read first (serde's `Deserialize` otherwise). |
//!
//! # A method's return type is its contract (0.7)
//!
//! | signature | cardinality | schema `returns` / `error` / `progress` |
//! |---|---|---|
//! | `-> T` | one | T / — / — |
//! | `-> Result<T, E>` | one | T / E / — |
//! | `-> impl Stream<Item = T>` | many | T / — / — |
//! | `-> impl Stream<Item = Result<T, E>>` | many | T / E / — |
//! | `-> impl Stream<Item = Progress<T, P>>` | many | T / — / P |
//! | `-> impl Stream<Item = Result<Progress<T, P>, E>>` | many | T / E / P |
//!
//! A Result item travels as data, `{"ok": t}` or `{"err": e}`; a
//! `Progress::Update(p)` as a progress frame. The namespace is the type's
//! name — `impl Foo`, or the stem of `impl FooActivation` — or the trait's,
//! on `impl Foo for Type` (MACRO-13, RETURNS-47; `namespace = "..."`
//! overrides it).
//! `#[method(returns(..))]` and `#[derive(StreamEvent)]` are gone, and
//! `#[method(streaming)]` is a deprecated no-op on a stream form. See the
//! README's per-macro reference.
//!
//! Deprecated surfaces still accepted: `#[hub_methods]`, `#[hub_method]`, the
//! `hub` and `children = [...]` flags on `#[activation]`. See the migration
//! section of the README.
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
mod parse;
mod request;
mod type_first;


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
                // RETURNS-8: the removed attributes fail here too, not only
                // inside `#[activation]`.
                match &meta {
                    Meta::List(list) if list.path.is_ident("returns") => {
                        return Err(syn::Error::new_spanned(&list.path, parse::RETURNS_REMOVED));
                    }
                    _ => {}
                }
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
/// #[plexus::activation]                    // namespace "bash", implicitly
/// impl Bash {
///     /// Execute a bash command
///     #[plexus::method]
///     async fn execute(&self, command: String) -> impl Stream<Item = BashEvent> {
///         // ...
///     }
/// }
/// ```
///
/// # The return type is the contract (RETURNS-8)
///
/// Six forms: `-> T`, `-> Result<T, E>`, `-> impl Stream<Item = T>`,
/// `-> impl Stream<Item = Result<T, E>>`, `-> impl Stream<Item = Progress<T,
/// P>>`, `-> impl Stream<Item = Result<Progress<T, P>, E>>`. A plain return
/// yields one item, `impl Stream` many; `Result` and `Progress` are
/// recognised by name. Any other return type is a compile error that lists
/// the six. `returns(..)` is removed (a compile error); `streaming` is a
/// deprecated no-op on a stream form and an error on a one-item one.
///
/// # No in-call question (ASK-1)
///
/// `bidirectional` is removed (a compile error): a method never asks its
/// caller mid-call, and Plexus has no in-call or protocol-wide
/// confirmation. A method that wants its caller's yes builds it from its
/// own parameters (a `dry_run`, a plan id).
///
/// # Authorization arguments (R-1 / AUTHZ-CORE-2)
///
/// - `scope = "..."` — explicit required scope, emitted into the generated
///   `MethodSchema.requires_credential`. Repeatable: `scope = "x.y",
///   scope = "x.z"` means the caller must satisfy **ALL** listed scopes
///   (conjunction — AND, not OR). Literals are validated at compile time
///   against the AUTHZ-CORE-4 scope grammar (dot-separated `[a-z][a-z0-9_]*`
///   segments, optional trailing `.*` segment wildcard, or bare `*`).
/// - `input_window = N` (BIDIR-6) — on a method with an `Input<T>` parameter,
///   the credit it grants (default 64). `input_max_buffered = N`: the bytes
///   held past which it grants no credit until drained (default 64 MiB). `input_max_items = N`,
///   `input_max_bytes = N`: limits over the call's life, none unless given.
///   An `Input<T>` is at most one, top-level, and not one of the params:
///   `MethodSchema.input` is `T`'s schema, and the client takes a stream.
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
/// `list = method` and `search = method` name the sibling methods that
/// stream a dynamic child's names (`#[plexus_macros::child(list = names)]`).
/// NEST-17: by path; the string form (`list = "names"`) is accepted for one
/// release with a deprecation warning.
///
/// # How the child inherits its caller (MACRO-20)
///
/// `forward = same_service | identity_only | pass_through | anonymous`
/// picks the policy the emitted `ChildRouter::forward_policy_for` hands
/// the child. A static child defaults to `same_service`: the same service as
/// its parent, it keeps the caller whole (principal stamp, session,
/// ceiling). Write `#[plexus_macros::child(forward = identity_only)]` for a
/// deputy that must not see the caller's session. A dynamic child keeps the
/// framework default (`IdentityOnly`) unless it names one.
///
/// Used standalone (outside an `#[activation]` block) this attribute is a
/// no-op that returns the function unchanged.
/// NEST-7: an activation returned by name — a peer, in this process or
/// across the network, typed by a contract's peer trait or untyped:
///
/// ```ignore
/// #[plexus_macros::peer(list = integrations)]
/// async fn integration(&self, name: &str) -> Result<Box<dyn forge::ForgePeer>, PeerError> { … }
/// #[plexus_macros::peer(list = dendrites, search = find)]
/// async fn dendrite(&self, name: &str) -> Result<Box<dyn Peer>, PeerError> { … }
/// ```
///
/// Inside `#[plexus_macros::activation]` the slot's role is
/// `MethodRole::Peer { interface }`, routing calls it (a typed peer checked
/// against the slot's interface first) and a failure is its typed error
/// item. Outside it this attribute does nothing.
#[proc_macro_attribute]
pub fn peer(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

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
/// - `namespace = "..."` - The activation namespace (the name rule below
///   gives it from `impl FooActivation`)
/// - `version = "..."` (optional, default: "1.0.0") - Version string
/// - `description = "..."` (optional) - Activation description
/// - `crate_path = "..."` (optional, default: "crate") - Path to substrate crate
/// - `client` (optional flag) - also emit the typed client (MACRO-2, below)
/// - `server_cfg = "<cfg predicate>"` (optional) - compile the server half
///   only under that `cfg` (MACRO-2, below)
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
///
/// # The typed client (`client`, `server_cfg`) — MACRO-2, 0.7 shapes RETURNS-9
///
/// The macro already knows, when it runs, every method's wire name, its
/// parameters and its precise contract (cardinality, item, error, progress).
/// With `client` it also emits a **typed client** for the activation, and the
/// activation's **aggregate** `{Ns}Event` (one variant per method, carrying
/// that method's item), so a Rust consumer never hand-writes a call site or
/// hand-mirrors an item type:
///
/// ```ignore
/// #[plexus_macros::activation(client)]                 // namespace "tickets", implicitly
/// impl Tickets { /* unchanged */ }
///
/// // emitted beside the impl: TicketsClient<T>, and TicketsEvent
/// let client = TicketsClient::new(transport);                      // any plexus_client::Transport
/// let health = client.health().await?;                             // one item: Health
/// match client.get("MNG-2".into()).await? { Ok(t) => .., Err(no) => .. }   // one, fallible
/// let rows: Vec<_> = client.list(None, None).try_collect().await?; // many: MethodStream
/// let mut s = client.import(src).with_progress();                  // Frame<item, P>
/// let mut ev = client.watch(None).events();                        // TicketsEvent
/// ```
///
/// `?` is the call failing (`ClientError::Remote(RemoteError)` for the
/// server's typed error frame); a match on the item is the activation's
/// answer. The client is generic over the transport, so the emitted code
/// names none of them; its parameter and item types are the activation's
/// own, so a change to either is a compile error in every consumer. A crate
/// that already defines a type named `{Ns}Event` renames it before turning
/// `client` on.
///
/// **Requires** the consuming crate to depend on `plexus-client`. A
/// proc-macro crate cannot check that for you at expansion time in general,
/// but the path is resolved through `proc-macro-crate`, so a missing
/// dependency is reported at the `client` flag rather than as an unresolved
/// import in code you did not write.
///
/// **`server_cfg = "..."`** puts the *server* half of the expansion — the
/// impl block, the method enum, the `Activation` impl, the jsonrpsee trait —
/// behind a `cfg`, while the client half stays unconditional. That is what
/// makes an activation crate reachable from `wasm32-unknown-unknown`:
///
/// ```ignore
/// // Cargo.toml: server deps are optional and behind `feature = "server"`.
/// // plexus-macros itself stays non-optional — a proc-macro crate is built
/// // for the host, so it costs a wasm32 build nothing.
/// #[plexus_macros::activation(
///     namespace = "tickets",
///     client,
///     server_cfg = "feature = \"server\"",
/// )]
/// impl Tickets { /* .. */ }
/// ```
///
/// A consumer then says one line — `tickets-api = { version = "..",
/// default-features = false }` — and gets the types and the client, with no
/// tokio, jsonrpsee or plexus-core anywhere in its tree. Gating your own
/// struct definition and server-only imports is still yours to do; the macro
/// only gates its own output.
///
/// Both flags are opt-in: an activation that names neither expands exactly
/// as it did before.
///
/// # `#[activation]` needs nothing — MACRO-13, RETURNS-47
///
/// The type names everything, every key is an override, and (NEST-17) the
/// trait is emitted every time, `{Name}ActivationSchema` unless named:
///
/// ```ignore
/// /// Tickets.                              // the activation's description
/// #[plexus_macros::activation]
/// impl Tickets {
///     /// Every ticket.                     // the method's description
///     pub async fn list(&self) -> impl Stream<Item = Ticket> + Send + 'static { .. }
///     #[plexus_macros::method]
///     pub async fn total(&self) -> u64 { .. }    // one item: marked
///     pub fn new() -> Self { .. }           // not async: a plain method
///     async fn helper(&self) -> u32 { .. }  // not pub: a plain method
/// }
/// // namespace "tickets"; `TicketsClient<T>`, whose `total()` answers
/// // `Result<u64, ClientError>` and `list()` a `MethodStream`; the trait
/// // `TicketsActivationSchema`, implemented by `Tickets` (when `Clone`).
/// ```
///
/// - **The name rule.** `impl Tickets` is named by itself (RETURNS-47: no
///   suffix forced); `impl TicketsActivation` by its stem, `Tickets`, as
///   before. The namespace is the name in snake_case (`TicketsApi` →
///   `tickets_api`), the client `TicketsClient<T>`. `namespace = "…"`
///   overrides the namespace.
/// - **The trait every time** (NEST-17, owner 2026-11-09, reversing
///   RETURNS-47's "only when named"). `impl Shop` and `impl ShopActivation`
///   emit `trait ShopActivationSchema`; `trait = "Bar"` renames it (and
///   names the client `BarClient` and, absent `namespace`, the namespace
///   `bar`); a contract is its trait (`Foo`). Each wire method is in it
///   under its own return type, boxed, with nothing the server injects. The
///   activation implements it by cloning itself into each stream (the
///   default trait only where the type is `Clone`); a method that takes
///   `auth`, `reach` or a `#[from_auth]` value reads it from the call's
///   context there (`plexus_core::call::caller()`, as `impl Trait for Type`
///   does — a missing caller is the method's error: its `E` from a
///   `PlexusError` or a `PeerError`; `reach` outside a call is
///   `Reach::nowhere()`). An `#[activation_param]` is in no context, so an
///   activation with one does not implement its trait. The client's
///   methods answer `Result<T, ClientError>` (one item) or the
///   `{Ns}{Method}Stream` alias (many); it implements the trait too when
///   every method answers one `Result<T, E>` with `E: From<PeerError>`, so
///   `dyn ShopActivationSchema` is local or remote alike.
/// - **`impl Trait for Type`.** `#[plexus_macros::activation] impl
///   forge::Forge for GitHub` serves a trait written by hand: the trait
///   names the namespace (`forge`) and the client (`ForgeClient<T>`, emitted
///   here), the type names nothing, and every `async fn` on `&self` is a
///   wire method, called as `<GitHub as forge::Forge>::m`. Its methods reach
///   their caller through the call's context, `plexus_core::call::caller()`
///   (a trait's signature cannot carry `#[from_auth]`). Under `auth =
///   "required"` the dispatch refuses an anonymous caller (or a call outside
///   any context) before the body of every method not `public`, out of band
///   and typed, as `#[from_auth]` does. `#[plexus_macros::from_auth(self.
///   access.verify)]` on the method is `#[from_auth]`'s form here: the
///   verifier runs on the signed-in caller before the body, its error the
///   call's typed refusal by the same route, its value the body's through
///   `plexus_core::call::verified::<User>()`. It has no direct JSON-RPC
///   methods, only the service's `{ns}.call`; `#[child]` in it is not served
///   yet (GRAFT-1).
/// - **Every public method a wire method.** Every `pub async fn` on `&self`
///   returning `impl Stream<Item = E>` is on the wire with no `#[method]`
///   (which stays for its overrides, and marks a one-item method); one
///   returning anything else unmarked is a compile error; `methods = marked`
///   restores the `#[method]`-only rule.
/// - **`served = written`** (KEEL-27, opt-in per block): an unmarked fn is
///   what its signature says — not `pub`, or not on `&self`, a helper; a
///   `pub fn x(&self) -> A` a mount, `pub fn x(&self, name: &str) ->
///   Option<A>` a slot (each checked by the compiler, `A: ChildRouter`, the
///   refusal naming the fn and what a mount returns); a `pub async fn x(&self,
///   name: &str) -> Result<Box<dyn I>, PeerError>` a fallible mount (a
///   `#[peer]`); any other `pub async fn` on `&self` a method (a bare
///   `#[method]`, one item or a stream). A marked fn is what its marker
///   says; an option no signature carries (`#[method(name = …)]` and the
///   rest, `#[child(list = …, search = …, forward = …, opaque)]`, a mount
///   behind a type alias, a private served fn) keeps its marker. Not with
///   `methods = marked`, nor on `impl Trait for Type`.
/// - **The client without a flag**, whenever the crate depends on
///   `plexus-client`; `client` is accepted with a deprecation warning.
/// - **The `server` feature default.** A crate whose manifest declares a
///   `server` feature gets its server half under `#[cfg(feature =
///   "server")]`; `server_cfg` overrides. The trait and the client are never
///   gated.
/// - **A bodiless activation is a contract**, and a contract is its trait:
///   when every wire method is bodiless (`pub async fn f(&self, ..) ->
///   impl Stream<..>;`), on `impl FooActivation` (trait `Foo`) or with
///   `trait = "…"`, the macro emits `pub struct FooActivation<D: Foo>(Arc<D>)`
///   delegating to `D`, `impl<D: Foo> Foo for FooActivation<D>`, and
///   `Foo::server(self)`: `service.register(Local::new().server())`.
///
/// **Written before MACRO-13.** An activation on a type not named
/// `…Activation` that says its `namespace` expands exactly as it did (no
/// trait; the client on the `client` flag; only `#[method]`s on the wire)
/// with a deprecation warning saying what to change. A later release
/// refuses it.
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

/// PERF.115-43 · an internally tagged enum's `Deserialize`, its tag read
/// first: in place of serde's, on a house event.
///
/// serde reads an internally tagged enum (`#[serde(tag = "type")]`, every
/// event the house writes) whole into a private buffer before it reads the
/// tag, then reads the variant out of the buffer. Written in serde's place,
/// this derive reads the first key: when it is the tag, the variant is chosen
/// by the tag's value and its fields are read straight from the rest of the
/// object; when it is not, serde reads the object, buffer and all, as it
/// always has. plexus-core writes `type` first, so a client, a hop or a
/// service reading another's events reads them without the buffer.
///
/// ```ignore
/// #[derive(Debug, Clone, serde::Serialize, plexus_macros::Deserialize, schemars::JsonSchema)]
/// #[serde(tag = "type", rename_all = "snake_case")]
/// pub enum TicketsEvent {
///     Health { version: String, count: u64 },
///     Ready,
///     Error { message: String },
/// }
/// ```
///
/// Everything else about the enum is serde's: the derive has serde derive
/// it twice more beside it (`#[serde(remote = …)]`), once as written and
/// once without its tag, so every `#[serde]` attribute (`rename_all`,
/// `rename`, `alias`, `other`, `default`, `skip`, `with`, `flatten`,
/// `deny_unknown_fields`, `expecting`) is read by serde's own code. A struct
/// variant and a unit variant are read straight; a newtype variant, a struct
/// variant with a flattened field and a variant read `with` a function are
/// read by serde's buffered read. What serde refuses is refused in serde's
/// words. A value inside a variant is read by the deserializer itself, so a
/// value only the input can carry (a `RawValue`, a map keyed by numbers)
/// reaches a variant whose tag comes first.
///
/// It is for an internally tagged enum that is not generic, with no
/// untagged variant (one is tried on the whole object once the tagged ones
/// fail, which needs the buffer); anything else is a compile error that says
/// to derive serde's `Deserialize`. The runtime is plexus-client's
/// (`plexus_client::__rt::type_first`), so the deriving crate depends on
/// plexus-client, as every crate with a generated client does.
#[proc_macro_derive(Deserialize, attributes(serde))]
pub fn deserialize_derive(input: TokenStream) -> TokenStream {
    type_first::derive(input)
}

/// PLEXUS-59 · `#[wire]` — one attribute for a type that crosses the wire.
///
/// A method's parameter, its answer or a stream's item is a type that serde
/// reads and writes and schemars describes. `#[wire]` derives all of it:
/// `Debug`, `Clone`, serde's `Serialize` and `Deserialize`, schemars'
/// `JsonSchema` — named through plexus-rpc when the crate depends on it, so
/// an app lists neither serde nor schemars for its wire types. An enum is
/// also tagged the way every Plexus event is, `#[serde(tag = "type",
/// rename_all = "snake_case")]`, unless it says its own `tag` or `untagged`.
///
/// ```ignore
/// use plexus_rpc::prelude::*;
///
/// #[wire]
/// pub enum CountEvent {
///     Tick { n: u32 },
/// }
/// ```
///
/// It takes no words; `#[serde(…)]` and `#[schemars(…)]` on the type and its
/// fields are read as ever. A type that needs fewer derives writes them out.
#[proc_macro_attribute]
pub fn wire(attr: TokenStream, item: TokenStream) -> TokenStream {
    if !attr.is_empty() {
        let attr = TokenStream2::from(attr);
        return syn::Error::new_spanned(attr, "#[wire] takes no words: write `#[wire]`, and `#[serde(…)]` below it for anything else")
            .to_compile_error()
            .into();
    }
    let input = parse_macro_input!(item as syn::DeriveInput);
    let (serde, schemars, crates) = match codegen::umbrella_crate(proc_macro2::Span::call_site()) {
        Some(u) => {
            let (s, j) = (format!("::{u}::__deps::serde"), format!("::{u}::__deps::schemars"));
            (quote! { ::#u::__deps::serde }, quote! { ::#u::__deps::schemars }, quote! { #[serde(crate = #s)] #[schemars(crate = #j)] })
        }
        None => (quote! { ::serde }, quote! { ::schemars }, quote! {}),
    };
    // An enum that names its own representation keeps it.
    let own = input.attrs.iter().filter(|a| a.path().is_ident("serde")).any(|a| {
        let mut said = false;
        let _ = a.parse_nested_meta(|m| {
            said |= m.path.is_ident("tag") || m.path.is_ident("untagged");
            if m.input.peek(syn::Token![=]) { let _: syn::Expr = m.value()?.parse()?; }
            Ok(())
        });
        said
    });
    let tag = match (&input.data, own) {
        (syn::Data::Enum(_), false) => quote! { #[serde(tag = "type", rename_all = "snake_case")] },
        _ => quote! {},
    };
    quote! {
        #[derive(Debug, Clone, #serde::Serialize, #serde::Deserialize, #schemars::JsonSchema)]
        #crates
        #tag
        #input
    }
    .into()
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
