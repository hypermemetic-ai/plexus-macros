# plexus-macros

The macros that turn an `impl` block into a [Plexus RPC](https://github.com/hypermemetic-ai/plexus-rpc)
activation: the attributes `#[activation]`, `#[method]`, `#[child]`, `#[peer]`
and `#[removed_in]`, and the derives `PlexusWire`, `PlexusRequest`,
`Credentials`, `HandleEnum` and `Deserialize`.

## Most people want plexus-rpc

[plexus-rpc](https://github.com/hypermemetic-ai/plexus-rpc) is the one
dependency a service takes. It brings these macros at its root and in its
prelude, with every crate their output names.

## Install

A crate that takes the macros by themselves lists the crates their output
names.

<!-- examples/counter/Cargo.toml -->
```toml
[dependencies]
plexus-macros = "0.11"
plexus-core = "0.11"
plexus-client = "0.11"
async-trait = "0.1"
futures = "0.3"
jsonrpsee = "0.26"
schemars = "1.1"
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
tokio = { version = "1.0", features = ["macros", "rt"] }
uuid = "1.0"
```

`#[activation]` writes the typed client when `plexus-client` is in the
manifest. `PlexusWire` names `serde` and `schemars`. plexus-macros and
plexus-core are taken from the same minor version.

## An example

<!-- examples/counter/src/lib.rs -->
```rust
use futures::{stream, Stream, StreamExt};
use plexus_macros::PlexusWire;

#[derive(Debug, Clone, PartialEq, PlexusWire)]
pub enum CountEvent {
    Tick { n: u32 },
}

#[derive(Clone, Default)]
pub struct Counter;

/// Counts.
#[plexus_macros::activation]
impl Counter {
    /// Ticks from 1 up to `to`.
    #[plexus_macros::method]
    pub async fn count(&self, /** the last tick */ to: u32) -> impl Stream<Item = CountEvent> {
        stream::iter(1..=to).map(|n| CountEvent::Tick { n })
    }
}
```

`Counter` is the activation `counter`, with the method `counter.count`. The
doc comments are its descriptions. The same expansion holds the trait
`CounterActivationSchema` and the client `CounterClient`:

<!-- examples/counter/src/lib.rs -->
```rust
let client = CounterClient::new(InProcessTransport::new(Counter));
let ticks: Vec<CountEvent> = client.count(3).try_collect().await.unwrap();
assert_eq!(ticks, [1, 2, 3].map(|n| CountEvent::Tick { n }));
```

`cargo test` runs this file, and holds this page to it.

## More

- [docs.rs/plexus-macros](https://docs.rs/plexus-macros)
- [CHANGELOG.md](CHANGELOG.md)
- [The activation](docs/activation.md): `#[activation]`, its names and its arguments.
- [Methods](docs/methods.md): parameters, the six return forms, `#[method]`'s arguments.
- [Children and peers](docs/children.md): `#[child]`, `#[peer]`, and an activation that names its parent.
- [The typed client](docs/client.md): what `#[activation]` writes for a Rust caller.
- [Wire types and derives](docs/wire-types.md): `PlexusWire`, `Deserialize`, `PlexusRequest`, `Credentials`, `HandleEnum`.
- [Callers and roles](docs/callers.md): the caller as a parameter, `public`, `roles(...)`, and what the gate enforces.
- [Deprecation](docs/deprecation.md): `#[deprecated]` and `#[removed_in]`, and the forms accepted with a warning.
- [plexus-client](plexus-client/README.md): the runtime of the typed client, in this repository.

MIT.
