# plexus-macros

Procedural macros for **Plexus RPC** activations. The macros are the **source of truth** for the client generation pipeline: your Rust function signature _is_ the schema.

---

## Overview: macros → schema → clients

```
#[plexus_macros::activation] Rust macros (this crate)
  ↓ schemars::schema_for!() + MethodRole / DeprecationInfo metadata
PluginSchema (serde JSON, pinned in plexus-core)
  ↓ Synapse (Haskell) fetches over Plexus RPC
Synapse IR (deduplicated, compiler-ready)
  ↓ plexus-codegen-{rust,typescript,python}
Client libraries (type-safe, documented, deprecation-aware)
```

Everything downstream — generated clients, OpenAPI docs, deprecation warnings in TypeScript IDEs — is derived from what `#[plexus_macros::activation]` emits into `PluginSchema`. Do not hand-write schemas; the macro owns them.

For **Rust** consumers there is a shorter path: the macro emits the client itself.

```
#[plexus_macros::activation(client)]   // impl Tickets — namespace "tickets", implicitly
  ↓  same expansion, same information
TicketsClient<T: plexus_client::Transport>   ← typed params, each method's precise item
TicketsEvent                                 ← the activation's aggregate, one variant per method
```

The Synapse → IR → codegen pipeline above stays the path for TypeScript, Python and everything non-Rust. See [The typed client](#the-typed-client-client--server_cfg).

---

## Documentation is primary: `///` everywhere

**`///` doc comments are the primary source of every description that reaches the wire.** The macros extract them at expansion time and emit them into `PluginSchema`. Generated clients (TypeScript, Rust, Python) render them as TSDoc / rustdoc / docstrings. Synapse shows them in the CLI tree.

| Where you write `///` | Where it appears on the wire |
|---|---|
| On the `impl` block annotated with `#[plexus_macros::activation]` | `PluginSchema.description` |
| On each `#[plexus_macros::method]` fn | `MethodSchema.description` |
| On each `#[plexus_macros::child]` fn | `MethodSchema.description` (role: `StaticChild` / `DynamicChild`) |
| On a struct / enum that derives `JsonSchema` | JSON Schema `description` for that type |
| On a field of such a struct | JSON Schema `description` for that field |
| On an enum variant | JSON Schema `description` for that variant |

**Parameters are the only exception** — individual parameter descriptions must use the `params(name = "...", other = "...")` attr on `#[plexus_macros::method]` (Rust has no per-argument doc-comment syntax).

**Precedence** — if both an explicit attr arg and a `///` doc comment are present, the explicit attr arg wins. In practice, prefer `///`; reach for the attr arg only when you need to diverge (e.g., a deliberately terser wire description than the internal Rust docs).

---

## A method's return type is its contract (0.7)

Write the narrowest signature that tells the truth; the macro reads
everything else from it — cardinality, item, error and progress — and never
needs a flag to know what a method does.

```rust,ignore
#[plexus_macros::activation(client)]                     // namespace "tickets", implicitly
impl Tickets {
    /// One item, can't fail.
    #[plexus_macros::method]
    pub async fn health(&self) -> Health { /* … */ }

    /// One item, can say no.
    #[plexus_macros::method(params(id = "Ticket id, e.g. MNG-2"))]
    pub async fn get(&self, id: String) -> Result<Ticket, GetError> { /* … */ }

    /// Many items, each can fail; an `Err` does not end the stream.
    #[plexus_macros::method]
    pub async fn list(&self, epic: Option<String>)
        -> impl Stream<Item = Result<ListItem, ListError>> + Send + 'static { /* … */ }

    /// Many items, with typed progress on the way.
    #[plexus_macros::method]
    pub async fn import(&self, source: String)
        -> impl Stream<Item = Result<Progress<ImportSummary, ImportStep>, ImportError>> + Send + 'static
    {
        stream! {
            yield Ok(Progress::Update(ImportStep::Chunk { done: 0, of: 10 }));
            yield Ok(Progress::Item(summary));
        }
    }
}
```

The full table — schema fields and the frames each item becomes — is in the
[per-macro reference](#the-six-signature-forms). `Progress` and `Note` are
`plexus_client::{Progress, Note}` (re-exported as `plexus_core::plexus::{Progress, Note}`).

---

## Minimal example — best practices

The smallest realistic activation. Uses `///` for every description, has one method with a `params(...)` attr, and exposes a static `#[plexus_macros::child]` into a nested namespace. **No `description = "..."` attr args anywhere** — the macros pick up doc comments automatically.

```rust,ignore
use async_stream::stream;
use futures::Stream;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A message echoed back to the caller.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EchoEvent {
    /// The message sent back verbatim, along with its length in characters.
    Echoed {
        /// The message echoed to the caller.
        message: String,
        /// Character count of `message`.
        length: usize,
    },
}

/// Echo service — returns messages verbatim and exposes a nested `stats` child.
#[derive(Clone)]
pub struct Echo;

impl Echo {
    pub const fn new() -> Self { Self }
}

/// Echo messages back verbatim; nested `stats` child exposes call introspection.
#[plexus_macros::activation(version = "1.0.0")]          // namespace "echo", implicitly
impl Echo {
    /// Echo the given message back to the caller exactly as received.
    #[plexus_macros::method(params(message = "The message to echo"))]
    async fn echo(&self, message: String) -> impl Stream<Item = EchoEvent> + Send + 'static {
        stream! {
            let length = message.chars().count();
            yield EchoEvent::Echoed { message, length };
        }
    }

    /// Nested stats child — reached as `echo.stats.<method>`.
    #[plexus_macros::child]
    fn stats(&self) -> EchoStats { EchoStats::new() }
}

/// Call-count introspection for the `echo` service.
#[derive(Clone, Default)]
pub struct EchoStats;

impl EchoStats {
    pub fn new() -> Self { Self }
}

/// Introspection for the Echo service — invocation counts and timing.
#[plexus_macros::activation(namespace = "stats", version = "1.0.0")]
impl EchoStats {
    /// Return the total number of `echo` calls served since startup.
    #[plexus_macros::method]
    async fn total(&self) -> u64 {
        0  // placeholder — wire up real state in production
    }
}
```

**What this demonstrates:**

- **`///` on the `impl` block** (line with `#[plexus_macros::activation]`) supplies `PluginSchema.description`. No `description = "..."` attr arg needed.
- **`///` on each method fn** supplies `MethodSchema.description`. Again, no `description = "..."` attr arg.
- **`params(message = "...")`** is the only way to document individual params — Rust has no per-argument doc-comment syntax.
- **`#[plexus_macros::child]` on a zero-arg accessor fn** creates a static child namespace. `synapse lforge echo stats total` routes into the nested activation.
- **Stream item type derives `JsonSchema`**, so its variant and field `///` doc comments flow through to the wire.
- **The namespace is implicit** for `Echo` (`echo`); `EchoStats` names its own (`stats`) because its implicit one would be `echo_stats`.
- **`total` returns a plain `u64`** — one item, so one data frame; `echo` returns a stream — many.

**Synapse invocations:**

```bash
synapse lforge echo echo --message "hello"      # → Echoed { message: "hello", length: 5 }
synapse lforge echo stats total                 # → 0
```

**Upgrade path from this minimal form:** add `params(...)` as more methods gain documented params; add more `#[plexus_macros::child]` accessors for nested namespaces; introduce `#[derive(PlexusRequest)]` + `request = ...` once you need auth / cookies / per-request context; add `#[deprecated]` + `#[plexus_macros::removed_in]` when rotating methods. Each of those is covered in the comprehensive example below.

---

## Comprehensive example

A single realistic activation that exercises every macro in the crate. Every attribute shown is supported on **0.7**. Features that are less common are called out inline.

> The example below uses several `description = "..."` attr args explicitly — partly for readability in a reference doc, partly because the comprehensive activation description is the easier way to show the `long_description` companion. **In your own code prefer `///` doc comments** per the [Documentation is primary](#documentation-is-primary--everywhere) section above.

```rust,ignore
use async_stream::stream;
use futures::Stream;
use plexus_macros::{HandleEnum, PlexusRequest};
use plexus_core::plexus::bidirectional::StandardBidirChannel;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

// -------------------------------------------------------------------
// 1. Stream item type — a plain domain enum.
//    The caller-wraps streaming architecture means you do NOT need a
//    `StreamEvent` derive. Standard serde + JsonSchema is enough.
// -------------------------------------------------------------------
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CounterEvent {
    /// Emitted on every tick.
    Tick { value: i64 },
    /// Emitted once when the counter is reset.
    Reset,
}

// -------------------------------------------------------------------
// 2. Typed handles — generated by `#[derive(HandleEnum)]`.
//    Handles are opaque strings on the wire; this derive makes them
//    typed on the Rust side. `plugin_id_type` pins the instantiation
//    when the owning activation is generic (IR-21).
// -------------------------------------------------------------------
#[derive(Debug, Clone, HandleEnum)]
#[handle(
    plugin_id = "Counter::PLUGIN_ID",
    // Optional — only needed if the activation is generic.
    // Pairs with `plugin_id` to emit `<Counter>::PLUGIN_ID`.
    plugin_id_type = "Counter",
    version = "1.0.0"
)]
pub enum CounterHandle {
    /// A single counter item.
    #[handle(method = "item", table = "items", key = "id")]
    Item {
        /// Stored item id.
        item_id: String,
    },
}

// -------------------------------------------------------------------
// 3. Typed request — generated by `#[derive(PlexusRequest)]`.
//    This struct describes the per-request HTTP context: cookies,
//    headers, query params, peer address, auth context. The
//    activation-level `request = CounterRequest` attr below wires it
//    into the dispatch path; fields become injectable via
//    `#[activation_param]` on method parameters.
// -------------------------------------------------------------------
#[derive(PlexusRequest)]
pub struct CounterRequest {
    /// Session cookie — required. Missing cookie → 401.
    #[from_cookie("access_token")]
    auth_token: String,

    /// Optional origin header. Absent → None.
    #[from_header("origin")]
    origin: Option<String>,

    /// Optional tenant query param: `?tenant=acme`.
    #[from_query("tenant")]
    tenant: Option<String>,

    /// Socket peer address, filled by the transport.
    #[from_peer]
    peer: Option<std::net::SocketAddr>,

    /// Full auth context (set by middleware).
    #[from_auth_context]
    auth: Option<plexus_core::plexus::AuthContext>,
}

// -------------------------------------------------------------------
// 4. The activation itself.
// -------------------------------------------------------------------
#[derive(Clone)]
pub struct Counter {
    // ... state, storage handles, etc.
}

impl Counter {
    pub fn new() -> Self { Self {} }
    fn lookup_item(&self, _id: &str) -> Option<Item> { None }
}

#[derive(Clone)]
pub struct Item; // stand-in for the child activation

#[plexus_macros::activation(
    namespace = "counter",
    version = "1.0.0",
    description = "Counter activation — demonstrates every plexus-macros feature",
    // Wire the typed request struct into dispatch. Every method gets
    // its fields available via `#[activation_param]` (see `whoami` below).
    request = CounterRequest,
)]
impl Counter {
    // -- A. A stream of items: the return type says "many" --
    /// Tick the counter and stream `CounterEvent`s.
    #[plexus_macros::method]
    async fn tick(&self, n: u32) -> impl Stream<Item = CounterEvent> + Send + 'static {
        stream! {
            for i in 0..n {
                yield CounterEvent::Tick { value: i as i64 };
            }
        }
    }

    // -- B. Parameter descriptions via `params(...)` --
    /// Add two numbers. `params(...)` documents each parameter.
    #[plexus_macros::method(
        params(
            a = "Left operand",
            b = "Right operand",
        )
    )]
    async fn add(&self, a: i64, b: i64) -> i64 {
        a + b    // a plain return: one item
    }

    // -- C. Deprecation with precise `removed_in` hint --
    //
    // Rust's `#[deprecated]` only knows about `since` and `note`;
    // the companion `#[plexus_macros::removed_in]` attribute supplies
    // the removal-version field on the emitted `DeprecationInfo`.
    /// Legacy method — retained for backward compatibility.
    #[deprecated(since = "0.5.0", note = "use `tick` instead")]
    #[plexus_macros::removed_in("0.6")]
    #[plexus_macros::method]
    async fn legacy(&self) -> impl Stream<Item = i64> + Send + 'static {
        stream! { yield 0; }
    }

    // -- D. HTTP method hint for REST clients --
    /// Read-only, safe to cache — mark it GET for generated HTTP clients.
    #[plexus_macros::method(http_method = "GET")]
    async fn status(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "ok".into(); }
    }

    // -- E. Bidirectional channel (streaming by convention) --
    //
    // The `ctx: &Arc<StandardBidirChannel>` argument is recognised by
    // the macro and not exposed as a param in the schema. For custom
    // request/response types, use
    // `bidirectional(request = "MyReq", response = "MyResp")`.
    /// Interactive prompt over a bidirectional channel.
    #[plexus_macros::method(bidirectional)]
    async fn prompt(&self, ctx: &Arc<StandardBidirChannel>)
        -> impl Stream<Item = String> + Send + 'static
    {
        let _ = ctx;
        stream! { yield "prompted".into(); }
    }

    // -- F. Request-struct field injection via `#[activation_param]` --
    //
    // The named parameter must match a field name on `CounterRequest`
    // by name AND type. The dispatch wrapper extracts the request
    // struct once per call and threads matching fields into the method.
    /// Reveal who the caller is (reads the `CounterRequest` fields).
    #[plexus_macros::method]
    async fn whoami(
        &self,
        #[activation_param] auth_token: String,
        #[activation_param] origin: Option<String>,
    ) -> impl Stream<Item = String> + Send + 'static {
        stream! {
            yield format!("token={} origin={:?}", auth_token, origin);
        }
    }

    // -- G. Per-method escape from activation-level request extraction --
    //
    // `request = ()` makes the dispatch skip `CounterRequest::extract()`
    // for this method. Use this for truly public endpoints like health
    // checks.
    /// Public health probe; no auth required.
    #[plexus_macros::method(request = ())]
    async fn health(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "ok".into(); }
    }

    // -- H. Static child (zero-arg) --
    //
    // `#[child]` methods are NOT RPC methods; they contribute to the
    // generated `ChildRouter` implementation. A zero-arg method is a
    // named static child routed by the method identifier.
    /// Global stats — reachable as `counter.stats.<method>`.
    #[plexus_macros::child]
    fn stats(&self) -> Stats { Stats {} }

    // -- I. Dynamic child with list + search capabilities --
    //
    // `fn NAME(&self, name: &str) -> Option<Child>` is a dynamic
    // fallback dispatcher. `list = "..."` and `search = "..."` name
    // sibling methods that stream child names for completion and
    // search — the generated `ChildRouter` advertises the
    // corresponding `ChildCapabilities` bits.
    /// Look up an item child (`counter.item <id>.<method>`).
    #[plexus_macros::child(list = "item_ids", search = "find_item")]
    async fn item(&self, id: &str) -> Option<Item> {
        self.lookup_item(id)
    }

    /// Stream every addressable item id.
    async fn item_ids(&self) -> impl Stream<Item = String> + Send + '_ {
        futures::stream::iter(vec!["a".into(), "b".into()])
    }

    /// Stream item ids matching `query`.
    async fn find_item(&self, query: &str) -> impl Stream<Item = String> + Send + '_ {
        let q = query.to_string();
        futures::stream::iter(vec![q])
    }
}

#[derive(Clone)]
pub struct Stats;
```

The example above is the **full** attribute surface of plexus-macros 0.7 (the six return-type forms are in the [per-macro reference](#the-six-signature-forms)). Real activations typically use a subset — see [`plexus-substrate`](https://github.com/hypermemetic/plexus-substrate/tree/main/src/activations) for concrete, minimally-documented usage (`echo` and `bash` are the smallest; `cone`, `claudecode`, and `solar` exercise children, handles, and generics).

---

## Request forwarding: `PlexusRequest` + `request = ...`

A **typed view of an inbound HTTP request**. Without it, your method sees only the RPC arguments; with it, you get cookies / headers / query params / peer address / auth context as first-class Rust fields — extracted, validated, and type-checked at dispatch time.

### What it is

`plexus_core::request::PlexusRequest` is a trait implemented by `#[derive(PlexusRequest)]`. It has two methods:

```rust,ignore
pub trait PlexusRequest: Sized {
    fn extract(ctx: &RawRequestContext) -> Result<Self, PlexusError>;
    fn request_schema() -> Option<serde_json::Value> { None }
}
```

`RawRequestContext` is the raw inputs — `http::HeaderMap`, `Uri`, optional `AuthContext`, optional `SocketAddr`. The derive generates both `extract` (run per-call by the dispatch code) and `request_schema` (included in the activation's `PluginSchema` with `x-plexus-source` hints).

### Why it exists

Auth tokens live in cookies. CSRF checks need the `Origin` header. Tenancy lives in query strings. Per-peer rate limits need the socket address. Before `PlexusRequest`, every activation had to hand-parse this out of a `RawRequestContext` — boilerplate, inconsistent errors, no schema. With the derive:

- **Typed extraction**: one `#[from_cookie("...")]` replaces several lines of header parsing.
- **Consistent errors**: missing required fields return `PlexusError::Unauthenticated`.
- **Schema metadata**: generated clients know which inputs come from cookies vs. body.
- **Composition**: the same struct is reused across every method of the activation.

### Field attributes

| Attribute | Source | Required? | Notes |
|---|---|---|---|
| `#[from_cookie("name")]` | Named `Cookie` header value | Non-`Option` → required (401 if missing) | Multi-value cookie headers are parsed correctly. |
| `#[from_header("name")]` | Named HTTP header | Non-`Option` → required | Case-insensitive per `http` crate. |
| `#[from_query("name")]` | Named URI query parameter | Non-`Option` → required | Uses `form_urlencoded::parse`. |
| `#[from_peer]` | `ctx.peer` (socket addr) | `Option<SocketAddr>` recommended | Transports without peer addr → `None`. |
| `#[from_auth_context]` | `ctx.auth` (`AuthContext`) | `Option<AuthContext>` recommended | Populated by auth middleware. |
| _(no attribute)_ | `PlexusRequestField::extract_from_raw` | Defined by the field type | Use for newtypes like `ValidOrigin`. |

`Option<T>` fields treat "absent" as `None` and never fail; non-`Option` fields return `PlexusError::Unauthenticated` on missing input. Parse failures (e.g. a cookie value that cannot parse into the declared type) also return `Unauthenticated`.

### Wiring it into an activation

The activation attribute takes a `request = <Type>` argument:

```rust,ignore
#[plexus_macros::activation(
    namespace = "my_ns",
    version = "1.0.0",
    request = MyRequest,
)]
impl MyActivation { /* ... */ }
```

Two ways to consume the extracted fields in methods:

1. **`#[activation_param]`** — the named parameter is pulled from the request struct by field name. The Rust type must match the declared field type (the macro enforces this):

   ```rust,ignore
   #[plexus_macros::method]
   async fn whoami(&self, #[activation_param] auth_token: String)
       -> impl Stream<Item = String> + Send + 'static { /* ... */ }
   ```

2. **`#[from_auth(expr)]`** — runs an arbitrary resolver expression against the extracted auth context. Useful when you want a _validated_ user type, not raw auth:

   ```rust,ignore
   #[plexus_macros::method]
   async fn admin(
       &self,
       #[from_auth(self.db.validate_admin)] admin: AdminUser,
   ) -> impl Stream<Item = String> + Send + 'static { /* ... */ }
   ```

To **opt a method out** of activation-level extraction (e.g. a public health probe), use `request = ()`:

```rust,ignore
#[plexus_macros::method(request = ())]
async fn health(&self) -> impl Stream<Item = String> + Send + 'static { /* ... */ }
```

### Wire-level: `x-plexus-source`

The derive augments the generated JSON Schema with a non-standard `x-plexus-source` key on each property, so generated clients know which input channel to use:

```json
{
  "type": "object",
  "properties": {
    "auth_token": {
      "type": "string",
      "x-plexus-source": { "from": "cookie", "key": "access_token" }
    },
    "origin": {
      "anyOf": [{ "type": "string" }, { "type": "null" }],
      "x-plexus-source": { "from": "header", "key": "origin" }
    },
    "tenant": {
      "anyOf": [{ "type": "string" }, { "type": "null" }],
      "x-plexus-source": { "from": "query", "key": "tenant" }
    },
    "peer": {
      "x-plexus-source": { "from": "derived" }
    }
  },
  "required": ["auth_token"]
}
```

### End-to-end example

Given this activation:

```rust,ignore
#[derive(PlexusRequest)]
struct AppRequest {
    #[from_cookie("access_token")] token: String,
    #[from_header("origin")]       origin: Option<String>,
}

#[plexus_macros::activation(namespace = "app", version = "1.0.0", request = AppRequest)]
impl App {
    #[plexus_macros::method]
    async fn me(&self, #[activation_param] token: String)
        -> impl Stream<Item = String> + Send + 'static
    {
        stream! { yield format!("token={}", token); }
    }
}
```

A request:

```
POST /rpc HTTP/1.1
Cookie: access_token=eyJhbGci...
Origin: https://app.example.com
Content-Type: application/json

{ "method": "app.me", "params": {} }
```

is dispatched as:

1. Transport builds a `RawRequestContext` from headers + URI.
2. Dispatch calls `AppRequest::extract(&raw)` → `AppRequest { token: "eyJhbGci…", origin: Some("https://app.example.com") }`.
3. The `token` field is threaded into `me` as its `token` parameter.
4. `me` runs; streamed output is serialised back over the transport.

### When to use `PlexusRequest` vs. method params

Rule of thumb:

- **Method params** — per-call inputs that belong to the business logic: `item_id`, `query`, `body`.
- **`PlexusRequest` + `#[activation_param]`** — per-request context that crosses every method: auth tokens, session cookies, tenancy, peer addr.

If a value would be the same for every method call in a request, it belongs in the request struct.

---

## The typed client (`client` + `server_cfg`)

The macro sees every method's wire name, its parameters and its precise
contract — cardinality, item, error, progress — at the moment it runs. With
the `client` flag it emits a **typed client** from that same information, and
the activation's **aggregate event type**, so a Rust consumer never
hand-writes a call site or hand-mirrors an item type.

```rust
#[plexus_macros::activation(client)]   // namespace "tickets", implicitly
impl Tickets {
    #[plexus_macros::method] pub async fn health(&self) -> Health { /* … */ }
    #[plexus_macros::method] pub async fn get(&self, id: String) -> Result<Ticket, GetError> { /* … */ }
    #[plexus_macros::method] pub async fn list(&self, epic: Option<String>)
        -> impl Stream<Item = Result<ListItem, ListError>> + Send + 'static { /* … */ }
    #[plexus_macros::method] pub async fn import(&self, source: String)
        -> impl Stream<Item = Result<Progress<ImportSummary, ImportStep>, ImportError>> + Send + 'static { /* … */ }
}
```

emits, beside the activation's own items:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", content = "event")]
pub enum TicketsEvent {                          // Pascal(namespace) + "Event"
    #[serde(rename = "health")] Health(Health),
    #[serde(rename = "get")]    Get(#[serde(with = "plexus_client::wire_result")] Result<Ticket, GetError>),
    #[serde(rename = "list")]   List(#[serde(with = "plexus_client::wire_result")] Result<ListItem, ListError>),
    #[serde(rename = "import")] Import(#[serde(with = "plexus_client::wire_result")] Result<ImportSummary, ImportError>),
}

pub struct TicketsClient<T> { /* .. */ }

impl<T> TicketsClient<T> {
    pub const NAMESPACE: &'static str = "tickets";
    pub fn new(transport: T) -> Self;
    pub fn transport(&self) -> &T;
    pub fn into_transport(self) -> T;
}

impl<T: plexus_client::Transport> TicketsClient<T> {
    pub const METHOD_GET: plexus_client::MethodRef;            // one per method

    // one item: an async fn returning it
    pub async fn health(&self) -> Result<Health, ClientError>;
    pub async fn get(&self, id: String) -> Result<Result<Ticket, GetError>, ClientError>;

    // many items: a MethodStream<'_, item, progress, aggregate>
    pub fn list(&self, epic: Option<String>)
        -> MethodStream<'_, Result<ListItem, ListError>, NoProgress, TicketsEvent>;
    pub fn import(&self, source: String)
        -> MethodStream<'_, Result<ImportSummary, ImportError>, ImportStep, TicketsEvent>;
}
```

Requires the consuming crate to depend on
[`plexus-client`](plexus-client/) — the wasm-safe runtime the emitted code
links against (and derives the aggregate through, so nothing else is needed
in `Cargo.toml`). A missing dependency is reported at the `client` flag, not
as an unresolved import in code you did not write.

### What you get

| method form (server) | client method | yields |
|---|---|---|
| one: `-> T` / `-> Result<T, E>` | `async fn m(&self, ..) -> Result<I, ClientError>` | `I` = `T` / `Result<T, E>` |
| many: `-> impl Stream<Item = …>` | `fn m(&self, ..) -> MethodStream<'_, I, P, {Ns}Event>` | `Result<I, ClientError>`; progress frames dropped undecoded |
| `.with_progress()` on a many call | — | `Result<Frame<I, P>, ClientError>`, in arrival order |
| `.events()` on a many call | — | `Result<{Ns}Event, ClientError>`, `{Ns}Event::M(item)` |

`I` is the method's item: `T`, or `Result<T, E>` decoded from its
`{"ok"}` / `{"err"}` wire form. `P` is its progress type — `Note` for
`Progress<T>`, the declared `P` for `Progress<T, P>`, and the uninhabited
`NoProgress` when the item is not a `Progress` (so `.with_progress()` exists
on every many call and never yields progress the schema does not declare).
A one-item method never reports progress, so it has no stream to opt in on.

**The one rule for a receiver: `?` is the call failing; a match on the item
is the activation's answer** (RETURNS-1 decision 8):

```rust
match client.get("MNG-2".into()).await? {                 // ? = the call failed (typed RemoteError)
    Ok(ticket) => show(ticket),
    Err(GetError::NotFound { id }) => missing(id),        // the activation said no
}

let rows: Vec<_> = client.list(Some("MNG".into())).try_collect().await?;   // a plain TryStream

let mut s = client.import(src).with_progress();          // opted in
while let Some(frame) = s.next().await {
    match frame? { Frame::Progress(ImportStep::Chunk { done, of }) => bar.set(done, of), Frame::Item(r) => finish(r) }
}

let mut all = futures::stream::select_all([client.list(None).events(), client.import(src).events()]);
while let Some(ev) = all.next().await { state.reduce(ev?) }   // ev: TicketsEvent
```

| | |
|---|---|
| **Typed params** | the method's own parameter types, in signature order. `#[activation_param]`, `#[from_auth]` and `auth:` params are absent — the server injects those. `Option` params that are `None` are **omitted**, not sent as `null`. |
| **Precise items** | each method's own item type — not a shared enum, not a copy. A Result method's err is a value (`Ok(Err(e))` in a stream) and does not end it. |
| **Typed call failures** | `ClientError::Remote(RemoteError)` — the server's error frame, `kind`-tagged and exhaustive (`method_not_found`, `invalid_params { param, problem }`, `unauthenticated`, `forbidden { reason }`, …) — vs `ClientError::Transport` (the connection failed), `Decode` (the answer does not fit the contract) and `Encode` (an argument would not serialize; nothing was sent). A failed call is an item of the stream, so a cut-off stream cannot pass for a finished one. |
| **The aggregate** | `{Ns}Event`: one variant per method, named Pascal(method), tagged with the wire name, carrying the item. Derived, never sent. Every `.events()` has the same type, so merging calls is ordinary stream plumbing. |
| **Transport-generic** | any `plexus_client::Transport` — in-process, WebSocket JSON-RPC, REST + SSE. The emitted code names none of them. |
| **Zero drift** | rename a field on an item type and every consumer of the client stops compiling. |

A one-item call reads the call to its `done` before it returns, so a
transport that cancels a dropped call (the WebSocket one unsubscribes) never
cancels one that finished; no item, or a second one, is a `Decode` error.

The aggregate derives `Debug` and `Clone`, so every item type of a `client`
activation implements both. **A crate that already has a type named
`{Ns}Event`** — the hand-written shared enum the aggregate replaces — renames
it before turning `client` on (RETURNS-1 pin 13).

Without the macro, the same decoding is `plexus_client::decode::<I>(items)`
(and `decode_result::<T, E>`, `decode_with_progress::<I, P>`) over a raw
`Transport::call` stream.

### Reaching wasm32: `server_cfg`

An activation crate is usually also a server, and tokio / jsonrpsee /
plexus-core do not build for `wasm32-unknown-unknown`. `server_cfg` puts the
**server half** of the expansion — the impl block, the method enum, the
`Activation` impl, the jsonrpsee trait — behind a `cfg`, while the client half
stays unconditional:

```toml
# Cargo.toml
[features]
default = ["server"]
server  = ["dep:plexus-core", "dep:tokio", "dep:jsonrpsee", "dep:serde_json", "dep:uuid", "dep:async-trait"]

[dependencies]
# NOT optional: a proc-macro crate is built for the host, never the target,
# so a wasm32 build pays nothing for it — and that is what lets the macro run,
# and emit the client, in a build with no server dependencies at all.
plexus-macros = "0.7"
plexus-client = "0.7"
serde         = { version = "1", features = ["derive"] }
```

```rust
#[plexus_macros::activation(
    namespace = "tickets",
    client,
    server_cfg = "feature = \"server\"",
)]
impl Tickets { /* .. */ }
```

A consumer then needs **one dependency line**:

```toml
tickets-api = { version = "0.1", default-features = false }
```

and gets the types and the client with no tokio, jsonrpsee or plexus-core
anywhere in its tree. Gating your own struct definition and server-only
imports is still yours to do — the macro only gates its own output.

Worked end to end in [`fixtures/tickets-api`](fixtures/tickets-api/) (the
activation) and
[`fixtures/tickets-wasm-consumer`](fixtures/tickets-wasm-consumer/) (a
consumer with no wire code of its own). Both build for
`wasm32-unknown-unknown`.

### Transports

| Transport | Where | Status |
|---|---|---|
| `MemoryTransport` | `plexus-client` | scripted test double; wasm-safe by construction |
| `InProcessTransport` | `plexus-core` (`plexus::in_process`) | drives an `Activation` directly — no server; a call it cannot dispatch is the typed error frame a hub would send |
| `WsTransport` | `plexus-client`, feature `ws-native` / `ws-web` | WebSocket JSON-RPC; one connection, subscriptions multiplexed |
| REST + SSE | — | not yet built |

`WsTransport::connect(url, hub_namespace)` is the wire transport, and the
only one that carries the full `StreamMetadata`. It is **behind a feature**
so that `plexus-client`'s default stays the four-dependency wasm-clean core:
`ws-native` builds on `tokio-tungstenite`, `ws-web` on `web_sys::WebSocket`,
and both expose the same type and the same constructor — a consumer picks
the backend in `Cargo.toml` and changes no code. The protocol itself (ids,
routing, cancellation, both frame shapes) is written once and shared, so the
backends cannot drift.

```rust
let transport = plexus_client::WsTransport::connect("ws://127.0.0.1:4455", "menger").await?;
let tickets = TicketsClient::new(&transport);   // …and reuse it for every activation
```

`hub_namespace` is the server's `DynamicHub::new(ns)` namespace, not an
activation namespace: the same `tickets.list` goes to `menger.call` on one
deployment and to another hub's `call` elsewhere. Worked end to end against a
real server by menger itself: its `tickets` activation carries `client` and
`server_cfg`, the browser app takes the crate with `default-features =
false`, and `tests/wire_client.rs` there drives the emitted `TicketsClient`
over `WsTransport` against the in-process server. (A client-only mirror of
menger's declarations lived under `fixtures/` while menger was still on the
0.5 line; it is gone — one declaration, nothing to drift.)

Both flags are opt-in: an activation naming neither expands exactly as before.

---

## Security model

This documents the **actual generated behavior**, not an aspiration. Read it before assuming a method is protected.

- **Auth is fail-open by default.** A method whose signature carries no auth signal — no `auth: &AuthContext`, no `auth: Option<&AuthContext>`, no `#[from_auth(...)]` resolver — is generated with **no auth check at all** and is callable **unauthenticated**. The dispatch arm only injects an auth check when the method requested one (`src/codegen/activation.rs`, the `auth_injection` logic in `generate_dispatch_arms`).
- **Default activation posture is `Optional` — it enforces nothing.** `auth = "required"` is opt-in and is the **only** fail-closed posture. Even under `required`, a method declaring `Option<&AuthContext>` is downgraded to a warning (it still receives `None` rather than being rejected).
- **Roles, scopes, and credential metadata are schema metadata only.** Declared roles/scopes and `#[credential(...)]` clauses are reflected into the schema for clients and tooling. They are **never enforced at dispatch** — the framework does not check them before invoking your method.
- **A `#[from_auth(expr)]` resolver must itself fail closed.** The macro splices `expr` verbatim into the dispatch arm and binds its result with `?`. It does **not** verify that the resolver actually denies unauthorized callers. If your resolver returns `Ok(...)` for an unauthenticated/under-privileged principal, the call proceeds. Enforcement is entirely the resolver's responsibility.

The practical consequence: authorization is opt-in, per-method, and only as strong as the code you write. The macro provides plumbing, not a policy.

---

## Per-macro reference

### `#[plexus_macros::activation(...)]`

Attached to an `impl` block to generate the activation's `Activation` trait impl, method enum, RPC trait, and (when `#[child]` methods are present) `ChildRouter` impl.

| Arg | Type | Default | Notes |
|---|---|---|---|
| `namespace = "..."` | string | implicit: snake_case(struct), a trailing `Hub` / `Activation` dropped | Activation namespace. Appears as `plugin_schema.namespace` and in every frame's `content_type` (`ns.method`). Only needed when the implicit one is wrong — see [the implicit namespace](#the-implicit-namespace). |
| `version = "..."` | string | `CARGO_PKG_VERSION` | Semver; used by handles and deprecation schemas. |
| `description = "..."` | string | _(none)_ | Max **15 words** — macro errors on overflow. Use `long_description` for more. |
| `long_description = "..."` | string | _(none)_ | Unbounded documentation; projected into `PluginSchema`. |
| `request = MyType` | type | _(none)_ | Typed request struct; see the [Request forwarding](#request-forwarding-plexusrequest--request--) section. |
| `resolve_handle` | flag | _(off)_ | Generates `Activation::resolve_handle` that delegates to `self.resolve_handle_impl(handle)`. You implement the `impl`. |
| `crate_path = "..."` | string | auto (via `proc-macro-crate`) | Override the `plexus-core` import path. Rarely needed after CHILD-6. |
| `plugin_id = "UUID"` | string | deterministic UUIDv5 from namespace | Explicit plugin UUID. Must parse as a UUID. |
| `namespace_fn = "method"` | string | _(none)_ | Advanced: generate `namespace()` as a method call instead of a constant. |
| `client` | flag | _(off)_ | Also emit the typed client — see [The typed client](#the-typed-client-client--server_cfg). Requires a `plexus-client` dependency. |
| `server_cfg = "<pred>"` | string | _(none)_ | Compile the **server** half of the expansion only under that `cfg` (e.g. `feature = "server"`). Requires `client`. This is what makes an activation crate reachable from wasm32. |
| `hub` | flag | _(off, deprecated)_ | Deprecated — hub mode is inferred from `#[child]` methods. Removed in 0.6. |
| `children = [ident, ...]` | ident list | _(empty, deprecated)_ | Deprecated — use `#[child]` methods. Removed in 0.6. |

### `#[plexus_macros::method(...)]`

Attached to methods inside an `#[activation]` impl. Marks the method as an RPC endpoint and collects schema metadata.

| Arg | Type | Default | Notes |
|---|---|---|---|
| `description = "..."` | string | first `///` doc-comment block (CHILD-5) | Explicit arg wins over doc comments. |
| `name = "..."` | string | method identifier | Override the RPC name without renaming the Rust fn. |
| `bidirectional` | flag | _(off)_ | Method uses `StandardBidirChannel`. Use `bidirectional(request = "...", response = "...")` for custom types. |
| `http_method = "..."` | string | _(none)_ | One of `GET\|POST\|PUT\|DELETE\|PATCH`. Hint for REST generators. |
| `params(name = "doc", ...)` | key/value list | _(empty)_ | Per-parameter descriptions. Less common now that field-level `///` extraction works; retained for cases where parameters have no natural Rust doc site. |
| `request = ()` | tuple | _(inherits)_ | Skip activation-level request extraction for this method (public endpoint escape hatch). |
| `request = OtherType` | type | _(inherits)_ | Reserved — per-method request type override is not yet wired end-to-end. |

**Removed in 0.7** (compile errors that name the replacement):

| Arg | Error says | Instead |
|---|---|---|
| `streaming` | "cardinality comes from the return type — remove `streaming`" | `-> T` / `-> Result<T, E>` is one item; `-> impl Stream<Item = …>` is many |
| `returns(Variant, ...)` | "give each method its own return type" | a per-method item type, e.g. `-> Result<Ticket, GetError>` |

**Method parameter attributes** (applied inside the `fn` signature):

- `#[activation_param]` — inject a field from the activation-level request struct; name + type must match.
- `#[from_auth(expr)]` — call `expr(&auth_context)` and bind the result; useful for validated user types.

Params are decoded one at a time, so a bad call names its param: a missing
required one is an error frame `invalid_params { param, problem: "missing" }`,
one of the wrong JSON type `invalid_params { param, problem: "malformed",
detail }`.

#### The six signature forms

A method's return type is its whole contract. The macro reads it and fills
`MethodSchema`'s `returns` (`T`), `error` (`E`), `progress` (`P`) and
`cardinality`, and dispatches each item to its frames:

| signature | cardinality | schema `returns` / `error` / `progress` | frames per item |
|---|---|---|---|
| `-> T` | one | T / — / — | data `content: T`, then done |
| `-> Result<T, E>` | one | T / E / — | data `{"ok": t}` or `{"err": e}`, then done |
| `-> impl Stream<Item = T>` | many | T / — / — | data `content: T` … done |
| `-> impl Stream<Item = Result<T, E>>` | many | T / E / — | data `{ok}` / `{err}` … done; an `Err` does not end the stream |
| `-> impl Stream<Item = Progress<T, P>>` | many | T / — / P | `Update(p)` → progress `content: p`; `Item(t)` → data `content: t` |
| `-> impl Stream<Item = Result<Progress<T, P>, E>>` | many | T / E / P | `Ok(Update(p))` → progress; `Ok(Item(t))` → data `{ok}`; `Err(e)` → data `{err}` |

- **An err is data.** `Err(e)` is the activation saying no, not the call
  failing; a stream carries on past it unless the method ends it (`yield
  Err(e); return;`). A call failure (bad params, unknown method, auth) is the
  error frame.
- **Progress needs a stream.** `Progress<T>` defaults `P` to `Note { message,
  percentage }`; `Progress<T, P>` declares it. `-> Progress<T>` (one item) is
  a compile error.
- **Recognition is syntactic**: a type whose last path segment is `Result`
  with two type arguments, or `Progress` with one or two. An alias
  (`anyhow::Result<T>`, `type R = …`) is a plain `T`. `impl Stream<Item = …>`
  may carry any extra bounds (`+ Send + 'static`), and `stream! { … }` bodies
  work as before.
- **Anything else is a compile error that lists the six forms** — notably
  `BoxStream<'_, T>`, `Pin<Box<dyn Stream<…>>>` and `PlexusStream`: return
  `impl Stream<Item = …> + Send + 'static` instead.
- A plain-`T` stream (`impl Stream<Item = OneEnum>`, 0.6's only form) compiles
  and sends exactly what it did.

Bidirectional methods, `#[child]` list/search methods, `#[activation_param]`
/ `#[from_auth]` parameters and `request = …` work with every form.

#### The implicit namespace

Without `namespace = "..."`, an activation's namespace is its struct's name
in snake_case with one trailing `Hub` or `Activation` removed:

| struct | namespace |
|---|---|
| `Tickets` | `tickets` |
| `HyperforgeHub` | `hyperforge` |
| `InfoActivation` | `info` |
| `ClaudeCode` | `claude_code` (write `namespace = "claudecode"` to keep a legacy name) |

`namespace = "..."` is used exactly as given.

### `#[plexus_macros::child]` / `#[plexus_macros::child(list = "...", search = "...")]`

Attached to methods inside an `#[activation]` impl to register them in the generated `ChildRouter`. Two accepted shapes:

| Shape | Role | Notes |
|---|---|---|
| `fn NAME(&self) -> Child` | Static child | Routing name is the method identifier. Sync or async. |
| `fn NAME(&self, name: &str) -> Option<Child>` | Dynamic fallback | Called for any name not matched by a static child. Sync or async. |

Optional arguments on dynamic `#[child]`:

| Arg | Type | Advertises capability | Sibling method shape |
|---|---|---|---|
| `list = "method"` | ident | `ChildCapabilities::LIST` | `(async) fn METHOD(&self) -> impl Stream<Item = String>` (or `BoxStream<'_, String>`) |
| `search = "method"` | ident | `ChildCapabilities::SEARCH` | `(async) fn METHOD(&self, query: &str) -> impl Stream<Item = String>` |

Sibling methods are validated at macro-expansion time — typos in `list = "..."` or a wrong signature become compile errors pointing at the attribute.

`#[child]` methods do **not** appear as RPC endpoints in the schema; they contribute to routing only. The presence of any `#[child]` method flips the activation into hub mode (replacing the deprecated `hub` flag).

### `#[plexus_macros::removed_in("X")]`

Companion to Rust's built-in `#[deprecated]`. Carries the removal-version field that rustc's `#[deprecated]` doesn't accept.

```rust,ignore
#[deprecated(since = "0.5", note = "use `tick` instead")]
#[plexus_macros::removed_in("0.6")]
#[plexus_macros::method]
async fn legacy(&self) -> impl Stream<Item = i64> + Send + 'static { /* ... */ }
```

The macro writes this into `MethodSchema.deprecation` as `DeprecationInfo { since, removed_in, message }`. Without the companion attribute, `removed_in` defaults to the string `"unspecified"`.

Applying `#[plexus_macros::removed_in]` to a method **without** `#[deprecated]` is a compile error.

### `#[derive(PlexusRequest)]`

See [Request forwarding](#request-forwarding-plexusrequest--request--) for the full treatment. Generates:

- `impl PlexusRequest for YourStruct` (extraction + schema).
- `impl schemars::JsonSchema for YourStruct` using `plexus_core::__schemars` (no direct `schemars` dependency required from the caller).

Supported field attributes:

```
#[from_cookie("name")]  #[from_header("name")]  #[from_query("name")]
#[from_peer]            #[from_auth_context]
```

Fields without any source attribute must implement `PlexusRequestField` — used for newtype wrappers that carry their own validation.

### `#[derive(HandleEnum)]`

Generates `to_handle()`, `impl TryFrom<&Handle>`, and `impl From<_> for Handle` for an enum that represents the set of typed handles an activation issues.

Enum-level `#[handle(...)]` arguments:

| Arg | Required? | Notes |
|---|---|---|
| `plugin_id = "CONST_OR_PATH"` | yes | Path to the activation's plugin UUID constant — e.g. `"Counter::PLUGIN_ID"` or `"MY_PLUGIN_ID"`. |
| `version = "X.Y.Z"` | yes | Semver for handles emitted by this enum. |
| `plugin_id_type = "Type"` | no | Concrete instantiation to qualify `plugin_id` when the owning activation is generic (e.g. `"Cone<NoParent>"`). Fixes E0283 ambiguity. |
| `crate_path = "..."` | no (`"plexus_core"`) | Override import path. |

Variant-level `#[handle(...)]` arguments:

| Arg | Required? | Notes |
|---|---|---|
| `method = "..."` | yes | The `handle.method` value on the wire. |
| `table = "..."` | no | SQLite table name (consumed by future handle-resolution tooling). |
| `key = "..."` | no | Primary key column. |
| `key_field = "..."` | no | Which variant field holds the key. Defaults to the first field. |
| `strip_prefix = "..."` | no | Strip this prefix before looking up by key. |

### `#[derive(Credentials)]`

Marks the credential-bearing fields of a response struct or enum so the
framework can mint, redact, and reflect them. Applied to the **item**
(struct / enum); individual fields carry an inner `#[credential(...)]`
helper attribute. The derive is additive — your type is re-emitted
unchanged; the macro emits sibling registry functions plus an
`impl HasCredentialMarkers` for the type.

A credential-bearing field's type **must** be `Credential<T>` (the sealed
wrapper from `plexus_auth_core`); the macro errors otherwise.

```rust,ignore
use plexus_macros::Credentials;
use plexus_auth_core::Credential;

#[derive(Credentials, Serialize, Deserialize, JsonSchema)]
pub struct LoginEvent {
    pub user_id: String,
    #[credential(
        kind = "bearer",
        attach_as = "header:authorization",
        scheme = "Bearer ",
        scopes = ["account.read", "account.write"],
    )]
    pub session: Credential<SessionToken>,
}
```

Field-level `#[credential(...)]` clauses:

| Clause | Required? | Notes |
|---|---|---|
| `kind = "..."` | yes | Closed `CredentialKind`: `bearer`, `cookie`, `oauth_access`, `oauth_refresh`, `oidc_id`, `aws_sts`, `macaroon`. Validated at compile time (a typo is a compile error). |
| `kind(other = "name")` | (alt) | Structured escape valve for backend-specific kinds. The bare `kind = "other"` form is rejected — use this parenthesized form. |
| `attach_as = "..."` | yes | Where the credential is projected on the wire. Prefixed-string grammar (see below). |
| `scheme = "..."` | no | Value prefix, e.g. `"Bearer "`. |
| `scopes = ["a.b", ...]` | no | Array of scope-name string literals. |
| `refresh_via = "ns.method"` | no | MethodPath (`<namespace>.<method>[.<sub>]*`, each segment `[a-z][a-z0-9_]*`). Validated at macro time. |
| `revoke_via = "ns.method"` | no | Same MethodPath grammar. |
| `sensitive` | no | Bare flag or `sensitive = true|false`. Accepted for clarity only — credentials are **always** treated as sensitive regardless. `expires_at` is rejected (it is set at mint time, not compile time). |

`attach_as` grammar (one of four prefixed-string forms):

| Form | Meaning |
|---|---|
| `"header:<name>"` | Project to the named response header (e.g. `"header:authorization"`). |
| `"cookie:<name>"` | Project to a `Set-Cookie` with the named cookie. |
| `"in_rpc_param:<name>"` | Carried in the named RPC parameter. |
| `"first_frame:<setup-method>:<param>"` | Supplied as `<param>` on the first frame of `<setup-method>`. |

What the macro emits (consumed downstream for schema + redaction):

- `__plexus_credential_marker_for_<Type>() -> Vec<CredentialFieldMarker>` —
  one entry per credential field, in declaration order, with all parsed
  metadata. For enums each entry carries its variant name.
- `__plexus_sensitive_fields_for_<Type>() -> Vec<(Option<&str>, &str)>` —
  the `(variant, field)` pairs treated as sensitive (mirrors the
  credential set, since credentials are always sensitive — used for
  log/serialization redaction).
- `impl HasCredentialMarkers for <Type>` — opts the type into the
  specialized marker-slice path read by `#[plexus_macros::method]` at
  schema-build time.

`#[derive(Credentials)]` cannot be applied to a union.

### `#[derive(StreamEvent)]` — **removed in 0.7**

The derive no longer exists. Item types are plain `Serialize + Deserialize + JsonSchema` types.

### `#[derive(JsonSchema)]` (compat shim)

Only available when plexus-macros is compiled with the `schemars-compat` feature **and** aliased as `schemars` in a crate's `dev-dependencies`. Produces a no-op `impl JsonSchema`, preventing a duplicate-impl conflict with the `impl JsonSchema` generated by `#[derive(PlexusRequest)]` when test code writes `#[derive(PlexusRequest, schemars::JsonSchema)]`.

Library crates almost never need this directly; the companion crate [`plexus-schemars-compat`](https://github.com/hypermemetic/plexus-schemars-compat) is the public entry point.

---

## Migration from deprecated surfaces

### 0.6 → 0.7: the return type is the contract

- **Drop `streaming`** — cardinality is the return type. A method that yields
  one value can now say so: `-> T` / `-> Result<T, E>`.
- **Drop `returns(..)`** — give the method its own item type instead of a
  filtered shared enum.
- **`namespace = "..."` is optional** — drop it when it equals the implicit
  namespace (snake_case of the struct, `Hub` / `Activation` dropped).
- **`#[derive(StreamEvent)]` is gone** — delete it; nothing replaces it.
- Methods returning `impl Stream<Item = OneEnum>` compile unchanged.
- `#[plexus_macros::activation(client)]`: a one-item method is an `async fn`
  returning its item (`m_all` is gone); a many-item method returns a
  `MethodStream` — `.try_collect()` where you called `m_all`, items directly
  where you matched `ClientEvent::Data(..)` (`ClientEvent` is gone; opt in to
  progress with `.with_progress()`, which yields `Frame`). A call failure is
  `ClientError::Remote(RemoteError)`, typed, instead of a message string.
  The flag now also derives `{Ns}Event`: rename a hand-written type of that
  name first.

### 0.5 → 0.6

All deprecations in plexus-macros 0.5 continue to compile with warnings through 0.5.x and are removed in 0.6.

### `#[hub_methods]` → `#[plexus_macros::activation]`

Before:

```rust,ignore
#[hub_methods(namespace = "bash", version = "1.0.0")]
impl Bash { /* ... */ }
```

After:

```rust,ignore
#[plexus_macros::activation(namespace = "bash", version = "1.0.0")]
impl Bash { /* ... */ }
```

Identical semantics. `#[hub_methods]` emits a deprecation warning pointing at the replacement.

### `#[hub_method]` → `#[plexus_macros::method]`

Before:

```rust,ignore
#[hub_method]
async fn execute(&self, command: String) -> impl Stream<Item = BashEvent> { /* ... */ }
```

After:

```rust,ignore
#[plexus_macros::method]
async fn execute(&self, command: String) -> impl Stream<Item = BashEvent> { /* ... */ }
```

### `hub = true` / `children = [...]` → `#[plexus_macros::child]` methods

Before:

```rust,ignore
#[plexus_macros::activation(namespace = "solar", version = "1.0.0", hub, children = [sun, earth])]
impl Solar { /* ... */ }
```

After:

```rust,ignore
#[plexus_macros::activation(namespace = "solar", version = "1.0.0")]
impl Solar {
    #[plexus_macros::child]
    fn sun(&self) -> Sun { /* ... */ }

    #[plexus_macros::child]
    fn earth(&self) -> Earth { /* ... */ }
}
```

Hub mode is now **inferred** from the presence of `#[child]` methods (CHILD-8). The `hub` flag and `children = [...]` list emit deprecation warnings and are scheduled for removal in 0.6.

---

## Output pipeline

The macro output feeds the same pipeline today as it did in 0.4, but with richer metadata:

```
#[plexus_macros::activation]  (this crate)
  ↓ emits
PluginSchema { methods: [MethodSchema { role, deprecation, params, ... }] }
  ↓ served over Plexus RPC (plexus-transport)
Synapse (Haskell) fetches, deduplicates, builds Synapse IR
  ↓ feeds
plexus-codegen-{rust,typescript,python}
  ↓ produces
Generated clients with typed methods, streaming iterators, and
deprecation warnings that mirror your `#[deprecated]` annotations
```

See the substrate reference server (`plexus-substrate`) for end-to-end examples of every feature in production use:

- `bash`, `echo` — minimal single-method activations.
- `cone`, `claudecode` — generic activations with `HandleEnum`, dynamic children, and `resolve_handle`.
- `solar` — hub activation with dynamic child routing + `list` capability.
- `interactive` — bidirectional channels.
- `changelog`, `mustache` — multi-method flat activations.

---

## License

MIT
