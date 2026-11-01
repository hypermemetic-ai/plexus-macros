# Changelog

All notable changes to plexus-macros and plexus-client are documented here.
This file starts at 0.7.0; earlier changes are in the git history.

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
