# Changelog

All notable changes to plexus-macros and plexus-client are documented here.
This file starts at 0.7.0; earlier changes are in the git history.

## [Unreleased]

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
