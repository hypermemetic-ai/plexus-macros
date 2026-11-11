# The activation

`#[plexus_macros::activation]` goes on an `impl` block and makes its type an
activation: a named group of methods a service serves.

<!-- tests/docs_examples.rs -->
```rust
#[derive(Clone)]
pub struct Tickets;

/// Tickets.
#[plexus_macros::activation]
impl Tickets {
    /// Every ticket.
    pub async fn list(&self) -> impl Stream<Item = Ticket> + Send + 'static {
        stream::iter([Ticket { id: "T-1".into() }])
    }

    /// How many there are.
    #[plexus_macros::method]
    pub async fn total(&self) -> u64 {
        1
    }

    pub fn new() -> Self {
        Self
    }
}
```

## What it writes

| item | what it is |
| -- | -- |
| `impl Activation for Tickets` | The namespace, version and description, the method list, the schema, and the dispatch of a call to its method. It comes from plexus-core. |
| `impl ChildRouter for Tickets` | Routing to the activation's [children](children.md). |
| `TicketsMethod` | An enum of the methods and their schemas: `TicketsMethod::method_schemas()`. |
| `TicketsRpcServer` | A jsonrpsee server trait of the methods that take nothing from the server, which `Activation::into_rpc_methods` answers. An `impl Trait for Type` activation has none. |
| `trait TicketsActivationSchema` | Every method under its own return type. See [the trait](#the-trait). |
| `TicketsClient<T>`, `TicketsEvent` | The [typed client](client.md) and the aggregate of its events. Written when `plexus-client` is a dependency of the crate. |

Every activation also serves a method named `schema`, which answers the
activation's schema, or one method's.

<!-- tests/docs_examples.rs -->
```rust
let tickets = Tickets::new();
assert_eq!(tickets.namespace(), "tickets");
assert_eq!(tickets.description(), "Tickets.");
assert_eq!(tickets.methods(), vec!["list", "total", "schema"]);

let by_its_trait: &dyn TicketsActivationSchema = &tickets;
assert_eq!(by_its_trait.total().await, 1);

let client = TicketsClient::new(InProcessTransport::new(tickets));
assert_eq!(client.total().await.unwrap(), 1);
```

An activation has at least one method. Which functions of the block are
methods is in [Methods](methods.md).

## Names

The type names the activation. `Foo` below is the name; the namespace is the
name in snake_case (`TicketsApi` is `tickets_api`).

| written | name | namespace | client | trait |
| -- | -- | -- | -- | -- |
| `impl Foo` | `Foo` | `foo` | `FooClient` | `FooActivationSchema` |
| `impl FooActivation` | `Foo` | `foo` | `FooClient` | `FooActivationSchema` |
| `impl FooActivation`, every method without a body | `Foo` | `foo` | `FooClient` | `Foo` |
| `trait = "Bar"` on either | `Bar` | `bar` | `BarClient` | `Bar` |
| `impl Bar for Type` | `Bar` | `bar` | `BarClient` | the trait implemented |

`namespace = "..."` replaces the namespace on `impl FooActivation`, beside
`trait = "..."`, and on `impl Bar for Type`. On `impl Foo` it selects an
expansion that is [deprecated](deprecation.md#also-accepted-with-a-deprecation-warning).

Where the client is not named after the type, as with `trait = "Bar"` on
`impl Foo`, the alias `FooClient` is written beside it: a parent names a
child's client by the child's type.

## Descriptions

The `///` comment on the `impl` block is the activation's description. The
`///` comment on a method is the method's. A doc comment before a parameter
is the parameter's. The doc comments of a wire type, its fields and its
variants are in its JSON Schema.

`description = "..."` in the attribute replaces the comment. On a method, the
comment is then the method's `long_description`.

## Arguments

Every argument is optional.

<!-- tests/docs_examples.rs -->
```rust
#[plexus_macros::activation(
    namespace = "shop",
    version = "2.0.0",
    description = "Things for sale",
    long_description = "Every item the shop sells, and what it costs.",
)]
impl StorefrontActivation {
```

| argument | default | what it does |
| -- | -- | -- |
| `namespace = "..."` | from the name | The activation's namespace. |
| `trait = "Name"` | `{Name}ActivationSchema` | Names the trait, the client (`NameClient`) and, without `namespace`, the namespace. Refused on `impl Trait for Type`. |
| `version = "..."` | the crate's `CARGO_PKG_VERSION` | The version in the schema. |
| `description = "..."` | the `///` comment | At most 15 words. More is a compile error that names `long_description`. |
| `long_description = "..."` | none | A longer text in the schema. |
| `methods = marked` | `methods = all` | Only functions marked `#[method]` are methods. See [Methods](methods.md#which-functions-are-methods). |
| `served = written` | off | Every unmarked function is what its signature says. See [below](#served--written). |
| `typed_params` | off | Refuses bare string parameters and unknown keys. See [Methods](methods.md#typed_params). |
| `request = Type` | none | The type every call's request is read into. See [Wire types](wire-types.md#plexusrequest). |
| `required(field, ...)` | none | Fields of the request type that every method keeps. A method of this activation that says `request = ()` is a compile error. |
| `auth = "required" \| "optional" \| "mixed" \| "none"` | `"optional"` | What the macro checks about its methods' callers. See [Callers](callers.md#the-auth-argument). |
| `roles(name = [...], ...)` | none | The roles the activation declares. See [Callers](callers.md#roles). |
| `parent = Client` | none | The client of the service this activation is grafted onto. See [Children](children.md#an-activation-that-names-its-parent). |
| `server_cfg = "<cfg predicate>"` | see [below](#the-server-half) | Compiles the server half under that `cfg`. |
| `plugin_id = "<uuid>"` | a UUID v5 of `<namespace>@<major version>` | The activation's id, the constant `PLUGIN_ID` of the type. A value that is not a UUID is a compile error. |
| `resolve_handle` | off | `Activation::resolve_handle` calls `self.resolve_handle_impl(handle)`, which the type provides. |
| `namespace_fn = "method"` | none | `namespace()` calls `self.method()` at run time. |
| `crate_path = "..."` | found in the manifest | The path the output names plexus-core by. |

A word the attribute does not take is a compile error that lists the words
it takes.

## The trait

The trait holds every method, without what the server passes in (`auth`,
`reach`, `#[from_auth]` values, `#[activation_param]` values, `parent`). A
method that answers one item returns `BoxFuture<'static, item>`. A method
that answers many returns `BoxStream<'static, item>`. An `Input<T>` parameter
is a `BoxStream<'static, T>`. Both box types are plexus-client's when it is a
dependency of the crate, and futures' otherwise. The trait is dyn-compatible.

The activation implements its trait when the type is `Clone`: each call
clones it into the future or stream it returns. A method that takes `auth`,
`reach` or a `#[from_auth]` value reads it from the context of the call it
runs in. The activation does not implement its trait when:

- a method takes an `#[activation_param]`;
- the `impl` block has generic parameters and names no trait (then no trait is written);
- a method answers one item that is not a `Result`, and needs a caller or a `parent`.

The client implements the trait too when every method answers one
`Result<T, E>` with `E: From<PeerError>` and takes no `Input`. A call that
fails on the way is then `E::from(PeerError)`. That impl is in the
[server half](#the-server-half).

## A contract

An `impl FooActivation` block whose methods have no bodies is a contract. No
struct is written by hand. The macro writes the trait `Foo`, the struct
`FooActivation<D: Foo>` that delegates every method to a `D`, and
`Foo::server(self)`, which makes one.

<!-- tests/docs_examples.rs -->
```rust
/// Greets people.
#[plexus_macros::activation]
impl GreeterActivation {
    /// Greet someone by name.
    pub async fn greet(&self, name: String) -> impl Stream<Item = String> + Send + 'static;
}

pub struct Polite;

impl Greeter for Polite {
    fn greet(&self, name: String) -> BoxStream<'static, String> {
        Box::pin(stream::iter([format!("Good day, {name}.")]))
    }
}
```

<!-- tests/docs_examples.rs -->
```rust
let activation: GreeterActivation<Polite> = Polite.server();
let client = GreeterClient::new(InProcessTransport::new(activation));
let said: Vec<String> = client.greet("Ada".into()).try_collect().await.unwrap();
assert_eq!(said, ["Good day, Ada."]);
```

`FooActivation::new(x)` and `FooActivation::from_arc(arc)` make one as well,
and `inner()` answers the implementation.

A contract's methods take only what a caller passes. A block that mixes
methods with and without bodies is a compile error. So is a contract on
`impl Foo`, whose trait would share the type's name: name the type
`FooActivation`, or say `trait = "..."`.

When every method of a contract answers one `Result<T, E>`, the macro also
writes `trait FooPeer`, and `FooClient::peer(transport)`, a remote
implementation of `Foo`. See [peers](children.md#peers).

## `impl Trait for Type`

A trait written by hand is served by implementing it under the attribute.
The trait names the namespace and the client. Every `async fn` on `&self` is
a method.

<!-- tests/docs_examples.rs -->
```rust
#[allow(async_fn_in_trait)]
pub trait Forge {
    async fn repos(&self) -> Vec<String>;
}

#[derive(Clone)]
pub struct GitHub;

/// GitHub, as a forge.
#[plexus_macros::activation]
impl Forge for GitHub {
    /// Its repositories.
    async fn repos(&self) -> Vec<String> {
        vec!["plexus-macros".into()]
    }
}
```

<!-- tests/docs_examples.rs -->
```rust
assert_eq!(GitHub.namespace(), "forge");
let client = ForgeClient::new(InProcessTransport::new(GitHub));
assert_eq!(client.repos().await.unwrap(), ["plexus-macros"]);
```

In this form:

- A method reaches its caller through the call's context,
  `plexus_core::call::caller()`. Its signature is the trait's and carries no
  `#[from_auth]` parameter.
- `#[plexus_macros::from_auth(expr)]` on the method runs `expr` on the
  signed-in caller before the body. Its error refuses the call. The body
  reads its value with `plexus_core::call::verified::<T>()`.
- Under `auth = "required"`, a caller who is not signed in is refused before
  the body of every method that is not `public`.
- `trait = "..."`, a method without a body, `#[child]` and `served = written`
  are compile errors.

## `served = written`

With `served = written`, a function without a marker is what its signature
says. A function with a marker is what the marker says.

| the function, unmarked | is |
| -- | -- |
| not `pub` | a helper, not served |
| `pub`, not on `&self` (`new`) | a helper, not served |
| `pub async fn x(&self, ...)` | a method |
| `pub fn x(&self) -> A` | a static child |
| `pub fn x(&self, name: &str) -> Option<A>` | a dynamic child |
| `pub async fn x(&self, name: &str) -> Result<Box<dyn I>, PeerError>` | a peer slot |

<!-- tests/docs_examples.rs -->
```rust
/// A shelf of books.
#[plexus_macros::activation(served = written)]
impl Shelf {
    /// Not on `&self`: a helper.
    pub fn new() -> Self {
        Shelf
    }

    /// Not `pub`: a helper.
    fn size(&self) -> u32 {
        1
    }

    /// A `pub async fn` on `&self`: a method.
    pub async fn count(&self) -> u32 {
        self.size()
    }

    /// A `pub fn` on `&self`: a static child.
    pub fn index(&self) -> Index {
        Index
    }

    /// A `pub fn` on `&self` taking a name: a dynamic child.
    pub fn book(&self, name: &str) -> Option<Book> {
        (name == "dune").then(|| Book { title: name.to_string() })
    }
}
```

The compiler checks that a child's type is an activation. Another type is
refused at the return type, in a message that names the function and what a
child returns. A `pub fn` on `&self` that returns nothing, or takes anything
but one `name: &str`, is a compile error.

What a signature cannot say keeps its marker: `#[method(...)]` with
arguments, `#[child(list = ..., search = ..., forward = ..., opaque)]`, a
served function that is not `pub`. A function
that a marked child names as its `list` or `search` stays a helper.

`served = written` beside `methods = marked` is a compile error.

## The server half

The expansion has two halves. The server half is the `impl` block, the
`Activation` and `ChildRouter` impls, the method enum and the activation's
impl of its trait. The trait and the client are the other half.

A crate whose manifest declares a feature named `server` gets the server
half under `#[cfg(feature = "server")]`. `server_cfg = "..."` names another
predicate. The trait and the client are never gated. A build without the
feature holds the types and the client, and no plexus-core. This is how an
activation's crate is a dependency of a `wasm32-unknown-unknown` build:

```toml
tickets-api = { version = "0.1", default-features = false }
```

The macro gates its own output. The crate gates its own struct and its
server-only imports. `fixtures/tickets-api` and
`fixtures/tickets-wasm-consumer` are such a crate and its consumer.

## What the output names

The server half names plexus-core, and `async_trait`, `futures`, `jsonrpsee`,
`schemars`, `serde`, `serde_json`, `tokio` and `uuid` by those names, so the
crate lists them. In a crate that depends on `plexus-rpc`, all of them are
named through it and the crate lists none.

plexus-core is found in the manifest, under the name the manifest gives it. A
crate with neither plexus-core nor plexus-rpc gets a compile error that says
so. `crate_path = "..."` sets the path by hand.
