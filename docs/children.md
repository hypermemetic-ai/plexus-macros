# Children and peers

An activation holds other activations. A child's methods are reached by its
path: `solar.sun.name`, `solar.mars.name`.

## `#[child]`

`#[plexus_macros::child]` goes on a function of an
[activation](activation.md)'s `impl` block that answers a child.

| the function | is | reached as |
| -- | -- | -- |
| `fn sun(&self) -> Child` | a static child | `<parent>.sun` |
| `fn planet(&self, name: &str) -> Option<Child>` | a dynamic child | `<parent>.<name>` |

Either is `fn` or `async fn`, `pub` or not. `Child` is an activation that is
`Clone`. A dynamic child is asked for every name no static child has. A
block has at most one dynamic child.

<!-- tests/docs_examples.rs -->
```rust
/// The solar system.
#[plexus_macros::activation]
impl Solar {
    /// How many planets.
    #[plexus_macros::method]
    pub async fn count(&self) -> u32 {
        2
    }

    /// The star: reached as `solar.sun`.
    #[plexus_macros::child]
    fn sun(&self) -> Sun {
        Sun
    }

    /// A planet by name: reached as `solar.<name>`.
    #[plexus_macros::child(list = planet_names, search = find_planets)]
    fn planet(&self, name: &str) -> Option<Planet> {
        ["earth", "mars"].contains(&name).then(|| Planet { name: name.to_string() })
    }

    fn planet_names(&self) -> impl Stream<Item = String> + Send + '_ {
        stream::iter(["earth".to_string(), "mars".to_string()])
    }

    fn find_planets(&self, query: &str) -> impl Stream<Item = String> + Send + '_ {
        let query = query.to_string();
        stream::iter(["earth".to_string(), "mars".to_string()].into_iter().filter(move |n| n.contains(&query)))
    }
}
```

A child is not a method. It is in the schema's method list by its role,
`static_child` or `dynamic_child`, with its `///` comment as its description,
and it is not in `Activation::methods()`. `#[child]` beside `#[method]` on
one function is a compile error.

The parent describes its static children, and theirs, by path
(`ChildRouter::router_static_child_schemas`), so a service knows
`solar.sun.name` as a method of its own.

### Arguments

| argument | on | what it does |
| -- | -- | -- |
| `list = function` | a dynamic child | Names a function of the block that streams the children's names: `(async) fn f(&self) -> impl Stream<Item = String>`, or `-> BoxStream<'_, String>`. `ChildRouter::list_children` answers the static children's names, then these. |
| `search = function` | a dynamic child | Names a function that streams the names matching a query: `(async) fn f(&self, query: &str) -> impl Stream<Item = String>`, or `-> BoxStream<'_, String>`. `ChildRouter::search_children` answers them. |
| `forward = policy` | either | How much of its caller a call into the child keeps. See [below](#what-a-child-sees-of-the-caller). |
| `opaque` | either | The child's type has no generated client. See [below](#in-the-client). |

A `list` or `search` function that is not in the block, or has another
signature, is a compile error at the argument. Both names are in the
schema: the role's `list_method` and `search_method`.

### What a child sees of the caller

A call routed into a child runs in a context made from the caller's by a
forward policy. The macro writes `ChildRouter::forward_policy_for`, which
answers each child's policy.

| `forward = ...` | plexus-core's policy | the child's context holds |
| -- | -- | -- |
| `same_service` | `SameService` | the caller whole: its user, its session and its metadata |
| `anonymous` | `Anonymous` | no caller |
| `identity_only` | `IdentityOnly` | what plexus-core's `IdentityOnly` keeps |
| `pass_through` | `PassThrough` | what plexus-core's `PassThrough` keeps |

A static child without `forward` gets `same_service`. A dynamic child
without it gets plexus-core's default. The policy is a bare word or a string.

## In the client

The parent's [client](client.md) has an accessor for every child. A call
through it goes to the child's path, and accessors nest.

<!-- tests/docs_examples.rs -->
```rust
let service = Arc::new(DynamicHub::new("svc").register(Solar));
let solar = SolarClient::new(HubTransport::anonymous(service));

assert_eq!(solar.count().await?, 2);
assert_eq!(solar.sun().name().await?, "Sol"); // solar.sun.name
assert_eq!(solar.planet("mars").name().await?, "mars"); // solar.mars.name
```

| child | accessor |
| -- | -- |
| static `fn x(&self) -> X` | `x()` answers `XClient` |
| dynamic `fn x(&self, name: &str) -> Option<X>` | `x(name)` answers `XClient` |
| `#[child(opaque)]` | `x()` or `x(name)` answers `plexus_client::Untyped` |
| an untyped peer slot | `x(name)` answers `plexus_client::Untyped` at `<path>.x.<name>` |
| a typed peer slot | `async x(name)` answers `Result<Client, PeerError>`: the contract's client at `<path>.x.<name>` |

The child's client is named from the child's type: `Planet` and
`PlanetActivation` are `PlanetClient`. In a crate with plexus-client, a
child whose type has no client and is not `opaque` is a compile error that
says so. So is a method whose name is a child's: they share one path.

A name that cannot reach a dynamic child (empty, holding a `.`, outside
`[a-z][a-z0-9_]*`, or a static child's name) does not fail the accessor.
Every call through it fails with `ClientError::InvalidName`, and nothing is
sent.

## Peers

`#[plexus_macros::peer]` goes on a function that answers an activation by
name and can fail:

<!-- tests/docs_examples.rs -->
```rust
/// A greeter by name, which may be away.
#[plexus_macros::peer]
pub async fn greeter(&self, name: &str) -> Result<Box<dyn Peer>, PeerError> {
    match name {
        "here" => Ok(Box::new(Greeter)),
        _ => Err(PeerError::NotFound { name: name.into() }),
    }
}
```

The function is `async`, takes one `name: &str` and returns
`Result<Box<dyn I>, PeerError>`. A peer is reached under its slot and its
name:

<!-- tests/docs_examples.rs -->
```rust
let here = lobby.greeter("here");
assert_eq!(here.path(), "lobby.greeter.here");
let said: Vec<serde_json::Value> = here.call("hello", json!({ "name": "Ada" })).try_collect().await.unwrap();
assert_eq!(said, [json!("hello, Ada")]);
```

`Box<dyn Peer>` is an untyped slot: any activation. `Box<dyn forge::ForgePeer>`
is a typed slot: `ForgePeer` is the peer trait of the
[contract](activation.md#a-contract) `Forge`, which the macro writes when
every method of the contract answers one `Result<T, E>` with
`E: From<PeerError>`. A typed slot's peer is checked against the contract's
interface before a call is routed to it, and one that does not serve it is
refused with `PeerError::ShapeMismatch`. `ForgeClient::peer(transport)` and
`ForgeClient::peer_at(transport, path)` make a remote implementation of
`Forge` from a transport, checked the same way.

`#[peer]` takes `list = function` and `search = function` as `#[child]` does.
The slot is in the schema with the role `static_child`, marked `peer: true`,
beside its interface when it is typed.

## An activation that names its parent

An activation written to be grafted onto a service names that service's
client, and a method takes it:

<!-- tests/docs_examples.rs -->
```rust
/// Restocks the shop it is grafted onto.
#[plexus_macros::activation(parent = ShopClient)]
impl Restock {
    /// What is running low.
    pub async fn low(&self, parent: Parent<ShopClient>, below: u32) -> impl Stream<Item = String> + Send + 'static {
        let items: Vec<String> = parent.items().filter_map(|item| async move { item.ok() }).collect().await;
        stream::iter(items.into_iter().take(below as usize))
    }
}
```

`parent` is `plexus_core::plexus::Parent<ShopClient>`. The server passes it:
it is no parameter of the call, and the client's `low` takes `below` alone.
A method takes at most one.

The macro reads the method's body. Every chain of calls that starts at
`parent` is a need: `parent.items()` is `shop.items`, and
`parent.ui().slots().list()` is `shop.ui.slots.list`. The needs are the
schema's `needs`, each with the hash of the method's shape.

<!-- tests/docs_examples.rs -->
```rust
let schema = Restock.plugin_schema();
let needs = schema.needs.unwrap();
assert_eq!(needs.iter().map(|need| need.method.as_str()).collect::<Vec<_>>(), ["shop.items"]);
assert!(!needs[0].hash.is_empty());
```

`parent` stays the start of a chain. Passed to a function, bound to another
name or cloned, it is a compile error at that use, unless the method lists
what it calls: `#[plexus_macros::method(uses("shop.items"))]`. A call
through a dynamic child's accessor (`parent.planet("earth").mass()`) is
named as written, `shop.planet`, with no hash.
