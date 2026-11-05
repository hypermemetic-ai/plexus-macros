# Changelog

All notable changes to plexus-macros and plexus-client are documented here.
This file starts at 0.7.0; earlier changes are in the git history.

## [Unreleased]

RETURNS-46, after 0.8.0: breaking, so the next release is 0.9.0.

### Added

- RETURNS-46: the aggregate up the tree. `{Ns}Event` gains one variant per
  `#[child]` carrying the child's aggregate — a static child's
  `{"method": "<segment>", "event": <child's>}`, a dynamic child's
  `{"method": "<child>", "event": {"name": "<instance>", "event": <child's>}}`,
  its event boxed and its name an `Arc<str>` — and `From<ChildEvent>` per
  static child. A dynamic child gets an accessor,
  `fn item(&self, name: &str) -> ItemClient<&T, Named<L, ItemEvent, TicketsEvent>>`,
  reaching `<path>.<name>.<method>`; a name that is empty, holds a `.`, is
  outside the service's path grammar (`[a-z][a-z0-9_]*`) or is a static
  child's segment makes every call through it fail at once with
  `ClientError::InvalidName` (`NameRefusal`), sending nothing; a name that
  is a method of the parent is the instance. `#[child(opaque)]`: a child with no client — no accessor, no
  variant, routed as before. plexus-client: `Lift` (`Root`, `Static`,
  `Named`), `Wrap`, `EventOf`, `Aggregated`, `HasClient`, `Route`, `RoutePath`,
  `MethodStream::lifted`, `ClientError::InvalidName`. A method whose wire
  name is a child's segment is refused. An activation without children: its
  aggregate unchanged.
- A method with an input half (BIDIR-6) is lifted and named like any other:
  its stream is `{Ns}{Method}Stream<'a, L>`, its `MethodRef` takes the
  client's `RoutePath`; `MethodStream::call_as_with_input`.

### Changed

- RETURNS-46 breaks, each for a reason:
  - **A generated client is `XClient<T, L = Root>`**, and a child accessor's
    return type names its lift: `tickets.inputs()` is
    `InputsClient<&T, Static<Root, __TicketsClientInputs>>` (was
    `InputsClient<&T>`). Code that names it: let it be inferred, or name the
    child's own client (`InputsClient::at(t, "tickets.inputs")`).
  - **`.events()` on a call through an accessor yields the ancestor's
    aggregate** — `tickets.inputs().answer(..).events()` yields
    `TicketsEvent::Inputs(InputsEvent::Answer(..))`, not `InputsEvent`. Code
    matching the child's own event there stops compiling; for the child's own
    aggregate call through the child's own client, `InputsClient::new` /
    `InputsClient::at`.
  - **A cycle of static children across types is a compile error**
    (E0391, "cycle detected when computing layout of `PingEvent`"): a
    dynamic child's event is boxed, a static child's is not, so a cycle made
    of static children alone has an aggregate of infinite size. No service
    could serve one: its schema is a tree with no bottom, which the service
    recurses into without end when it describes its static children (one
    activation that is its own static child, `-> Self`, is boxed and
    compiles — and overflows the stack in `DynamicHub::register`). The
    macro cannot name the cycle: each activation is expanded alone, and a
    cycle across types is visible to none of them. Break it with a dynamic
    child.
  - **A child must have a client, or be `#[child(opaque)]`** — now a dynamic
    child too (it had no accessor before). The error says so.
  - **`MethodRef`'s path is a `RoutePath`** (a `&'static str` or an
    `Arc<str>`), so `MethodRef` is `Clone`, not `Copy`; `MethodRef::at` takes
    `impl Into<RoutePath>` (a `&'static str` still works) and `path()`
    returns `&str`. `XClient::at` takes `impl Into<RoutePath>`; `path()`
    returns `&str`. `__rt::join_path` is gone: no path is interned any more.
  - **A generated many-item method returns
    `MethodStream<'_, I, P, XEvent, L, __XClientM>`** (was
    `MethodStream<'_, I, P, XEvent>`): `L` the client's lift, `__XClientM` a
    zero-sized `EventOf` emitted per method — its variant, inlined with the
    lift (through a `fn` pointer and then the lift, a static level cost ~10
    ns an event, about 10%). Code that names the type: use the method's
    alias, `{Ns}{Method}Stream<'a, L = Root>` (`RootTickStream<'a>`;
    `{Ns}{Method}MethodStream` when an item type is already called that),
    which each method returns and its doc names — or name `L` and `W` as `_`. `MethodStream::call` / `new` with a
    `fn(I) -> A` are as they were (`W` defaults to it); `call_as` takes an
    `EventOf`. A static child's lift is `Static<L, __XClientChild>`, a
    zero-sized `Wrap` per static child.
  - **A generated client is no longer `Copy`**: it is `Clone`, `Debug` and,
    for `XClient<T>`, `Default`.
  - `MemoryTransport::params_of` takes `impl Borrow<MethodRef>`.

### RETURNS-47: no suffix forced, a trait only when named, `impl Trait for Type`

Owner, 2026-11-04 (option 2), reversing MACRO-13's `always` and superseding
RETURNS-43. Breaking.

#### Added

- `#[plexus_macros::activation] impl Tickets` — a type not named
  `…Activation`, nothing in the attribute — is a whole declaration: namespace
  `tickets`, client `TicketsClient`. MACRO-13 refused it.
- `#[plexus_macros::activation] impl forge::Forge for GitHub`: a trait
  written by hand, served. The trait names the namespace (`forge`) and the
  client (`ForgeClient<T>`, emitted beside the impl); the type names nothing.
  Every `async fn` on `&self` is a wire method, any of RETURNS-8's forms,
  called as `<GitHub as forge::Forge>::m`. The trait needs no object safety
  to be served: rustc checks the concrete futures for `Send`. A method
  reaches its caller through the call's context
  (`plexus_core::call::caller()`), so `auth = "required"` asks no
  `#[from_auth]` of it. It has no direct JSON-RPC methods (only `{ns}.call`,
  where the context is). Refused, by name: `trait = "…"` beside it, a
  bodiless method, `#[child]` (GRAFT-1's).

#### Changed

- **No trait unless named.** `impl FooActivation` emits no `trait Foo` any
  more; `trait = "Foo"` emits it (implemented by the activation, as before),
  and a bodiless contract is still its trait. A bare `trait` key is refused
  ("a trait is emitted only when named"). What to do: add `trait = "Foo"`
  where the trait is used.
- **The client implements no trait.** Its methods answer
  `Result<T, ClientError>` (one item) or the `{Ns}{Method}Stream` alias
  (many), as their inherent methods did; nothing asks an item type for
  `FromClientError`, so `-> u64`, `-> bool`, `-> Vec<T>` compile. A client
  can no longer be a `dyn Foo` or served as a proxy with
  `FooClient::new(t).server()`: write the implementation that calls it, and
  says its failures as you want them (`tests/macro13_contract_tests.rs`,
  `Proxy`).
- `reach: Reach` is read with `current_reach()?` (plexus-core's, now
  fallible): outside a call the method is refused, never handed everywhere.
- The deprecation warning on an activation written before MACRO-13 (a type
  not named `…Activation` that says its `namespace`) says to drop
  `namespace`, or to name the type `…Activation` to keep it as an override —
  not that the suffix is needed.

#### Removed

- plexus-client: `into_events`, `error_events`, `trait_one`,
  `trait_one_with_input`, `trait_many`, `trait_progress`,
  `trait_result_progress`, `owned_call`, `owned_call_raw`,
  `owned_call_raw_with_input` — the client's half of the trait, nothing else
  used them. `FromClientError` stays (other code implements it) and is
  required by nothing; `BoxStream`, `BoxFuture`, `MaybeSend` stay.
- The fixtures' `from_client_error!` blocks.

## [0.8.0]

plexus-macros 0.8.0 and plexus-client 0.8.0, released with plexus-core 0.8.0:
the code these macros emit needs it.

### Changed (PUBLISH-3)

- The tests' `plexus-auth-core` dev-dependency asks for 0.3 (`CallCeiling`
  gained `max_buffered`, BIDIR-6).

BIDIR-6: `Input<T>`. BIDIR-17: `Input<Bytes>` as binary frames.

### Added (BIDIR-17)

- `MethodSchema.input` of an `Input<Bytes>` is marked binary (through
  plexus-core's `input::item_schema`).
- plexus-client: `InputItems` carries `InputItem` (`Json`, or `Binary` with the
  new `binary` feature, which `ws-native` and `ws-web` turn on; the default
  core stays four dependencies). `encode_input` makes a `Bytes` item `Binary`
  and needs `T: 'static`; `WsTransport` writes each as one binary frame.

### Added

- A method's `Input<T>` parameter: at most one, top-level, not in its params.
  `MethodSchema.input` is `T`'s schema. A second one, or one nested in a type
  the signature shows, is a compile error naming the method; one in a param's
  struct does not compile, since `Input` takes no serde.
- `#[method(input_window = N)]` (the credit, default 64),
  `input_max_buffered = N` (bytes held past which credit is held back,
  default 64 MiB; never a refusal), and the opt-in
  `input_max_items = N`, `input_max_bytes = N`.
- The emitted client takes the input as `impl Stream<Item = T>` in the
  parameter's place; the emitted trait takes it as a boxed stream.
- plexus-client: `Transport::call_with_input` (default: refused, naming the
  method), `InputItems`, `encode_input`, `MaybeSend`, `owned_call_with_input`.
  `WsTransport` feeds the input under credit as the returned stream is polled,
  spends `credit`, `ack` and `input_closed` itself (a caller never sees them;
  `StreamItem` is unchanged), and keeps items that arrive before their
  subscription id.
### Added

- PERF.115-43: `#[derive(plexus_macros::Deserialize)]`, an internally tagged
  enum's `Deserialize` that reads its tag first. Written where serde's
  `Deserialize` goes on a house event (`#[serde(tag = "type")]`), it reads the
  first key and, when it is the tag, reads the variant straight from the rest
  of the object, without serde's buffer; anywhere else, serde reads the object
  as before. Every `#[serde]` attribute is serde's own: the derive has serde
  derive the enum twice more (`remote`), as written and without its tag.
  Refused, by name: anything but an internally tagged enum, a generic one, one
  with an untagged variant. In a generated client a 64 B item falls from 6,558
  instructions to 6,081, a ticket row from 16,312 to 12,011; the event alone,
  as any Rust reader reads it, from 2,312 to 1,831 and 12,043 to 7,740.
  `plexus_client::__rt::type_first` is its runtime.
- PERF.115-36: a typed client decodes an item's content straight from the
  wire's text into the method's event, never through a `Value`.
  `Transport::call_raw` yields the call's items as a typed client reads them
  (`RawItem`, its content a `Content`); its default maps `call`'s items, so
  every transport has it, and `WsTransport` hands over the text unread.
  `decode_raw` and `owned_call_raw` are `decode` and `owned_call` for them.
  A generated client and a client's contract call these. A decode error is
  worded as `decode` words it. On a WebSocket from plexus-core, a 64 B item
  falls from 36,976 instructions to 6,404 in the client, a ticket row from
  77,577 to 16,145, a 17 KB ticket body from 176,873 to 128,388.

### Changed

- `WsTransport` reads each frame in place, not into a `Value`; a call that
  reads `StreamItem`s (`call`) gets the same items, built from their fields.
  A data notification as plexus-core writes it is read by its bytes for a
  typed call, and only its content is decoded. A frame it does not read so
  is read as a `Value`, as before. plexus-client takes serde_json's
  `raw_value` feature.

### Fixed

- An item of a `type` plexus-client does not know is skipped and the stream
  goes on (spec/03); it ended the stream.
- A notification that comes before the reply naming its subscription id is
  kept, while a call waits for its id, and handed over when the id arrives
  (spec/02 rule 4); it was dropped.

### Breaking (RETURNS-36: precise returns over main)

- A method's return type is its contract (RETURNS-8): `-> T`,
  `-> Result<T, E>`, `-> impl Stream<Item = T | Result<T, E> |
  Progress<T, P> | Result<Progress<T, P>, E>>`. `#[method(streaming)]` and
  `returns(..)` are compile errors; any other return type is one that lists
  the six forms. Under MACRO-13 every `pub async fn` on `&self` returning a
  form is a wire method.
- The client (RETURNS-9): a one-item method is an `async fn` returning
  `Result<item, ClientError>`; a many-item one a `MethodStream` (`.events()`,
  `.with_progress()`); `m_all`, `EventStream`, `ClientEvent`, `collect_data`
  are gone. `ClientError::Remote(RemoteError)` is typed. MACRO-16's `at`,
  `path` and child accessors are kept.
- The aggregate `{Ns}Event` (`{Ns}Aggregate` when an item type already has
  that name) is derived beside every client.
- MACRO-13's trait is always emitted, each method under its own return type
  (`BoxFuture` for a one-item method, `BoxStream` of its item for a stream);
  its client needs each item type to be `FromClientError`.
- plexus-client: `StreamItem::Progress { content_type, content }`;
  `StreamItem::Error` gains `error: RemoteError` beside `message`, `code`
  and the flattened spec/05 fields. `RemoteError` adds `TooManyCalls`,
  `ChildUnavailable`, `ChildTimeout` and `DenyReason::MissingGrant` (main's
  refusals).
- plexus-client's typed path (PERF-36's, main's): `RawItem::Progress {
  content_type, content }` and `RawItem::Error { message, code, recoverable,
  error }` follow the 0.8 frames; a `Decoder` takes the item's `Content`
  (`Content::decode::<T>`, `wire_result::from_content::<T, E>`), so a
  generated client's `MethodStream` and `call_one` decode from the wire's
  text. `decode_raw` yields a `MethodStream`. An error item is read whole
  through the `Value` path (it ends the call); data and progress in place.
- The implicit namespace of returns/0.7 (`Tickets` → `tickets`, a trailing
  `Hub` dropped) is not taken: MACRO-13's name rule stands.

## [0.7.0]

Release with plexus-core 0.7.0: the code these macros emit needs it, and
plexus-core 0.6.1 does not compile with it (E0599).

### Breaking

| Change | Before | After | What to do |
|---|---|---|---|
| A key no parameter claims, under `#[activation(typed_params)]` | ignored | `-32602` invalid params, `unknown field ...` naming the method's parameters | Send only declared keys |
| Emitted code and plexus-client | built on `Data.content: Value` | built on plexus-core's `Content` | Move to plexus-core 0.7.0; read content with `into_value` |

### Added

- `#[method(answer_by = "caller" | "person" | "grant:<activation.method>")]`.
- `plexus_client::watch`, several method streams as one.
- `WsTransport::connect_with_headers` (`ws-native`) and `WsTransport::adopt`
  (`ws-web`).

### Not enforced

- `answer_by` only declares who may answer, in the method's schema. Nothing
  emitted by the macro checks it. See plexus-core's changelog.
