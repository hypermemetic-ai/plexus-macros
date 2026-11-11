# The typed client

`#[activation]` writes a Rust client for the [activation](activation.md) it
is on, in a crate that depends on `plexus-client`. A crate without that
dependency gets no client and no error.

For `impl Tickets` it writes `TicketsClient<T>` and `TicketsEvent`, beside
the `impl` block. `T` is any `plexus_client::Transport`.

## Calling

Each method of the activation is a method of the client, with the same
parameters.

| the activation's method | the client's method | answers |
| -- | -- | -- |
| one item: `-> T`, `-> Result<T, E>` | `async fn m(&self, ...)` | `Result<item, ClientError>` |
| many items: `-> impl Stream<Item = ...>` | `fn m(&self, ...)` | a `MethodStream`: a stream of `Result<item, ClientError>` |

The item is the method's own: `T`, or `Result<T, E>`. `?` on the outer
`Result` is the call failing. The item is the activation's answer.

<!-- tests/docs_examples.rs -->
```rust
let client = TicketsClient::new(InProcessTransport::new(Tickets));

// One item: `?` is the call failing, the value is the answer.
let total: u64 = client.total().await?;
assert_eq!(total, 1);

// One item that can be an error: the activation's `Err` is a value.
match client.get("T-2".into()).await? {
    Ok(ticket) => panic!("found {ticket:?}"),
    Err(GetError::NotFound { id }) => assert_eq!(id, "T-2"),
}

// Many items: a stream of `Result<item, ClientError>`.
let all: Vec<Ticket> = client.list().try_collect().await?;
assert_eq!(all.len(), 1);

// An `Err` item is data, and the stream goes on past it.
let checked: Vec<Result<Ticket, GetError>> = client.check(vec!["T-2".into(), "T-1".into()]).try_collect().await?;
assert!(checked[0].is_err() && checked[1].is_ok());

// Progress is dropped unless asked for.
let imported: Vec<Result<Ticket, GetError>> = client.import("T-9".into()).try_collect().await?;
assert_eq!(imported, [Ok(Ticket { id: "T-9".into() })]);

// `.with_progress()` yields both, in the order they were sent.
let frames: Vec<Frame<Result<Ticket, GetError>, ImportStep>> = client.import("T-9".into()).with_progress().try_collect().await?;
assert_eq!(frames[0], Frame::Progress(ImportStep::Chunk { done: 1, of: 1 }));
assert_eq!(frames[1], Frame::Item(Ok(Ticket { id: "T-9".into() })));

// `.events()` yields the activation's aggregate, one variant per method.
let events: Vec<TicketsEvent> = client.list().events().try_collect().await?;
assert!(matches!(&events[0], TicketsEvent::List(Ticket { id }) if id == "T-1"));
```

- The parameters are the method's own types, in the order of its signature.
  What the server passes (`auth`, `reach`, `#[from_auth]`,
  `#[activation_param]`, `parent`) is absent.
- An `Option` parameter that is `None` is left out of the call, not sent as
  `null`.
- An `Input<T>` parameter is any stream of `T`.
- A one-item call reads the call to its end. No item, or a second one, is
  `ClientError::Decode`.
- `.with_progress()` and `.events()` are on a many-item call. A one-item
  method reports no progress.
- A many-item method's stream has a name to hold it by,
  `{Namespace}{Method}Stream<'a>`: `TicketsListStream<'a>`.

## When a call fails

A failed call is an `Err` item, so a stream that was cut off does not look
like one that ended.

| `ClientError` | what happened |
| -- | -- |
| `Remote(RemoteError)` | The service answered with an error: `MethodNotFound`, `InvalidParams { param, problem }`, `Unauthenticated`, `Forbidden { reason }`, `Execution { message }` and the others of `RemoteError`. |
| `Transport(TransportError)` | The connection failed. |
| `Decode { .. }` | The answer does not fit the method's types. |
| `Encode { .. }` | A parameter did not serialize. Nothing was sent. |
| `InvalidName { name, reason }` | A dynamic child's name cannot reach it. Nothing was sent. |
| `ShapeMismatch { .. }` | A node does not serve the interface the typed client was built against. |

## The client itself

<!-- tests/docs_examples.rs -->
```rust
type Client = TicketsClient<plexus_client::MemoryTransport>;
assert_eq!(Client::NAMESPACE, "tickets");
assert_eq!(Client::METHOD_LIST.qualified(), "tickets.list");
assert_eq!(TicketsClient::at((), "desk.tickets").path(), "desk.tickets");
```

| item | what it is |
| -- | -- |
| `TicketsClient::new(transport)` | A client that calls `tickets.<method>`. |
| `TicketsClient::at(transport, path)` | A client that calls `<path>.<method>`: an activation that is a child. |
| `NAMESPACE` | The activation's namespace. |
| `METHOD_<NAME>` | Each method's `plexus_client::MethodRef`. |
| `path()`, `transport()`, `into_transport()` | The path it calls, and its transport. |

`&T`, `Box<T>`, `Rc<T>` and `Arc<T>` of a transport are transports, so one
connection serves many clients: `TicketsClient::new(&transport)`.

## The aggregate

`TicketsEvent` has one variant for each method, named after the method in
PascalCase and holding the method's item. `.events()` on any many-item call
yields it, so calls of different methods merge into one stream of one type.

<!-- tests/docs_examples.rs -->
```rust
let event = TicketsEvent::List(Ticket { id: "T-1".into() });
assert_eq!(serde_json::to_string(&event).unwrap(), r#"{"method":"list","event":{"id":"T-1"}}"#);
let event = TicketsEvent::Get(Err(GetError::NotFound { id: "T-2".into() }));
assert_eq!(serde_json::to_string(&event).unwrap(), r#"{"method":"get","event":{"err":{"type":"not_found","id":"T-2"}}}"#);
```

The aggregate is made by the client and is not what travels: a call's items
travel as themselves. It derives `Debug`, `Serialize` and `Deserialize`, so
every item type of an activation with a client is `Debug`. Its name is
`{Namespace}Event`, or `{Namespace}Aggregate` when one of the activation's
item types is already named `{Namespace}Event`.

### Up the tree

An activation's aggregate also has one variant for each
[child](children.md), holding the child's aggregate. A static child's
variant holds it directly. A dynamic child's holds the instance's name and
the event in a `Box`.

<!-- fixtures/aggregate-tree/expected/aggregate.tree.jsonl -->
```json
{"method":"branch","event":{"method":"leaf","event":{"method":"fall","event":{"leaf":1}}}}
{"method":"branch","event":{"method":"leaf","event":{"method":"fall","event":{"leaf":2}}}}
{"method":"item","event":{"name":"a","event":{"method":"watch","event":{"seen":"a","n":1}}}}
```

A client reached through a parent's accessor yields the parent's aggregate.
One listener over a whole subtree is one merged stream, matched with no
wildcard:

<!-- fixtures/aggregate-tree/tests/aggregate.rs -->
```rust
let tree = TreeClient::new(&t);
let branch = tree.branch();
let leaf = branch.leaf();
let (a, b) = (tree.item("a"), tree.item("b"));
let mut all = futures::stream::select_all([
    tree.count(2).events(),
    branch.grow(3).events(),
    leaf.fall(2).events(),
    a.watch(2).events(),
    b.watch(1).events(),
]);
```

<!-- fixtures/aggregate-tree/tests/aggregate.rs -->
```rust
TreeEvent::Count(Tick { n }) => ticks += n,
TreeEvent::Branch(BranchEvent::Grow(Ok(Bud { bud }))) => buds += bud,
TreeEvent::Branch(BranchEvent::Grow(Err(GrowError::Frost { at }))) => frosts += at,
TreeEvent::Branch(BranchEvent::Leaf(FoliageEvent::Fall(Fell { leaf }))) => leaves += leaf,
TreeEvent::Item { name, event } => match *event {
```

A child's own client, `BranchClient::at(transport, "tree.branch")`, yields
the child's aggregate. `TreeEvent::from(branch_event)` lifts it one level.
That `From` is written for each static child. It is not written for a
dynamic child, for two static children of one type, or for a child that is
the activation itself.

A `#[child(opaque)]` and a peer slot have no variant.

## A node without a typed client

`plexus_client::Untyped` is a transport and a path, for a node whose
interface is not known when the program is written.

| method | what it does |
| -- | -- |
| `Untyped::new(transport, path)` | The node at `path`. |
| `call(method, params)` | A stream of `serde_json::Value` items. |
| `call_one(method, params)` | One `serde_json::Value`. |
| `child(name)`, `into_child(name)` | The node one level down. |
| `schema()` | The node's schema. |
| `typed::<XClient<_>>()` | The typed client, after checking that the node serves every method `XClient` calls, by the hash of each method's shape. Otherwise `ClientError::ShapeMismatch`. |
| `typed_unchecked::<XClient<_>>()` | The typed client, unchecked. |

## Transports

| transport | where | what it reaches |
| -- | -- | -- |
| `InProcessTransport::new(activation)` | `plexus_core::plexus` | One activation, in this process. |
| `HubTransport::anonymous(service)`, `HubTransport::as_(service, caller)` | `plexus_core::plexus` | A whole service in this process, children included. |
| `WsTransport::connect(url, service)` | `plexus_client`, feature `ws-native` or `ws-web` | A running service, over one WebSocket. `service` is the service's own name. |
| `MemoryTransport` | `plexus_client` | Nothing: it answers from a script, for a test. |

The client names no transport, so a crate that only calls a service depends
on plexus-client and the activation's types alone. See
[the server half](activation.md#the-server-half).
