# plexus-client

The runtime of the typed Rust client that `#[activation]` writes for a
[Plexus RPC](https://github.com/hypermemetic-ai/plexus-rpc) service: the
`Transport` trait, the stream of a call's items, the errors a call can end in,
and the WebSocket transports.

## Most people want plexus-rpc

[plexus-rpc](https://github.com/hypermemetic-ai/plexus-rpc) is the one
dependency a service takes, and it brings this crate with it. A crate takes
plexus-client by itself to call a service without serving one, in the browser
as well: it builds for `wasm32-unknown-unknown`.

## Install

<!-- examples/counter/Cargo.toml -->
```toml
plexus-client = "0.11"
```

| feature | what it adds |
| -- | -- |
| none | `Transport`, `MethodStream`, `ClientError`, `RemoteError`, `MemoryTransport`, `Untyped`, `watch`; four dependencies: `serde`, `serde_json`, `futures-core`, `futures-util` |
| `ws-native` | `WsTransport` over `tokio-tungstenite` |
| `ws-web` | `WsTransport` over the browser's `WebSocket` |
| `binary` | `InputItem::Binary`, a call's input sent as bytes |
| `schema` | `schemars::JsonSchema` for `Note`, `RemoteError`, `ParamProblem` and `DenyReason` |

## An example

A call goes through a `Transport` and answers a stream of items.
`MemoryTransport` answers from a script.

<!-- plexus-client/tests/readme_example.rs -->
```rust
const COUNT: MethodRef = MethodRef::new("counter", "count");

#[test]
fn a_call_is_a_stream_of_its_items() {
    let transport = MemoryTransport::new().script_events(COUNT, &[Tick { n: 1 }, Tick { n: 2 }]);

    let items = decode::<Tick>(transport.call(COUNT, json!({ "to": 2 })));
    let ticks: Vec<Tick> = futures::executor::block_on(items.try_collect()).unwrap();

    assert_eq!(ticks, [Tick { n: 1 }, Tick { n: 2 }]);
    assert_eq!(transport.params_of(COUNT), Some(json!({ "to": 2 })));
}
```

A call that fails is an `Err` item of its stream. `ClientError::Remote` is
the service's typed refusal, and `ClientError::Transport` is the connection.

<!-- plexus-client/tests/readme_example.rs -->
```rust
let items = decode::<Tick>(transport.call(COUNT, json!({ "to": 2 })));
let refused = futures::executor::block_on(items.try_collect::<Vec<Tick>>()).unwrap_err();

assert!(matches!(refused, ClientError::Remote(RemoteError::Unauthenticated)));
```

With `ws-native` or `ws-web`, `WsTransport::connect(url, service)` is the
transport to a running service, and a generated client takes it as it takes
any other: `CounterClient::new(transport)`.

## More

- [docs.rs/plexus-client](https://docs.rs/plexus-client)
- [CHANGELOG.md](../CHANGELOG.md)
- [The typed client](../docs/client.md): what `#[activation]` writes on top of this crate.
- [plexus-macros](../README.md): the macros, in this repository.

MIT.
