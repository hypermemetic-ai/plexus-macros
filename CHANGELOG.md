# Changelog

All notable changes to plexus-macros and plexus-client are documented here.
This file starts at 0.7.0; earlier changes are in the git history.

## [Unreleased]

BIDIR-6: `Input<T>`.

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
