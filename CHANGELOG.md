# Changelog

All notable changes to plexus-macros and plexus-client are documented here.
This file starts at 0.7.0; earlier changes are in the git history.

## [0.8.0] — unreleased (RETURNS-36: precise returns over main)

### Breaking

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
- MACRO-13's trait is emitted when every wire method is `impl Stream<Item =
  E>` of one `E` (its shape); an activation using the other forms has no
  trait. A contract (bodiless activation) must have the shape.
- plexus-client: `StreamItem::Progress { content_type, content }`;
  `StreamItem::Error` gains `error: RemoteError` beside `message`, `code`
  and the flattened spec/05 fields. `RemoteError` adds `TooManyCalls`,
  `ChildUnavailable`, `ChildTimeout` and `DenyReason::MissingGrant` (main's
  refusals).
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
