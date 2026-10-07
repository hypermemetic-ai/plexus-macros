# conformance: the RETURNS-7 fixture

One activation, `Conformance` (namespace `conformance`, implicit), with one
method per signature form (RETURNS-1). Under `expected/` is the JSON that
every generator of the plexus 0.7 line must reproduce. Rust is the canonical
implementation, and every other one is tested against these files. A change
to any file here is a contract change, and it names the RETURNS-1 decision it
follows.

| method | signature form | call | frames |
|---|---|---|---|
| `ping` | `-> Pong` | `{}` | data, done |
| `lookup` | `-> Result<Thing, LookupError>` | `{"id":"a1"}` / `{"id":"missing"}` | data `{ok}`, done / data `{err}`, done |
| `count` | `-> impl Stream<Item = Tick>` | `{"to":3}` | data ×3, done |
| `scan` | `-> impl Stream<Item = Result<Row, ScanError>>` | `{"prefix":"a"}` | data `{ok}`, data `{err}`, data `{ok}`, done |
| `load` | `-> impl Stream<Item = Result<Progress<Loaded, LoadStep>, LoadError>>` | `{"n":2}` | progress ×2 (`LoadStep`), data `{ok}`, done |
| `warm` | `-> impl Stream<Item = Progress<Warmed>>` | `{}` | progress ×2 (`Note`), data, done |
| call failures | | `frob {}` · `lookup {}` · `count {"to":"three"}` | one error frame each: `method_not_found` · `invalid_params` `missing` · `invalid_params` `malformed` |

## Files

- `src/lib.rs`, `src/types.rs`, `Cargo.toml`: the activation. It joins the
  plexus-macros workspace in RETURNS-8, the build that teaches the macro these
  forms. `types.rs` needs no macro, and plexus-core's test includes it.
- `expected/calls.json`: every call, as `{method, params, frames}`, where
  `frames` is relative to `expected/` (plus `line`, 1-based, for
  `errors.jsonl`).
- `expected/schema.json`: the `PluginSchema` the schema RPC returns: the six
  methods, then the `schema` method every activation's schema RPC lists
  (cardinality `one`; `returns` is `schema_for!(SchemaResult)` since
  RETURNS-26: this `PluginSchema`, or one `MethodSchema` when called with
  `{"method": "name"}`).
- `expected/wire/<method>[.<case>].jsonl`: the frames of one call, ending in
  its one `done`.
- `expected/errors.jsonl`: one error frame per call failure the fixture can
  provoke. Each line is a whole call, with no `done` after it.
- `expected/aggregate.jsonl`: every item of the `wire/` calls, in call order,
  as a serialized `ConformanceEvent`.
- `expected/ir.json`: the fixture's IR 3.0, added by RETURNS-13.

## Who round-trips what

| generator | ticket | reads | must |
|---|---|---|---|
| plexus-core (the files themselves) | RETURNS-7 | `expected/`, `src/types.rs` | every frame a `PlexusStreamItem` 0.7; the schema a `PluginSchema` 0.7 whose `returns`/`error`/`progress` are `schema_for!` of T/E/P; every Result item through `wire_result` (`plexus-core/tests/conformance_fixture.rs`) |
| macro, server half | RETURNS-8 | `src/`, `calls.json` | serve the activation in-process: its schema RPC equals `schema.json`, and each call in `calls.json` yields exactly its frames (`wire/`, or its `errors.jsonl` line) |
| macro, client half | RETURNS-9 | `src/`, `calls.json`, `wire/`, `errors.jsonl`, `aggregate.jsonl` | over `InProcessTransport`, each call decodes to the items `wire/` encodes (`load`'s `.with_progress()` yields its two `LoadStep`s; `scan` carries on past its err); `errors.jsonl` calls surface as `ClientError::Remote(RemoteError::…)` of that kind; the derived `ConformanceEvent` serializes them to `aggregate.jsonl` byte for byte, and back |
| plexus-protocol (Haskell) | RETURNS-12 | `schema.json`, `wire/`, `errors.jsonl` | parse, re-encode, equal (key order aside) |
| synapse IR 3.0 | RETURNS-13 | `schema.json` | the IR built from it equals `ir.json`, which RETURNS-13 writes |
| hub-codegen, TS | RETURNS-17 | `ir.json`, `wire/`, `errors.jsonl`, `aggregate.jsonl` | the generated client typechecks; replaying `wire/` decodes the items (`withProgress()`, `scan` as above); `errors.jsonl` throws `RemoteCallError` of that kind; `JSON.stringify` of each `.events()` value is its `aggregate.jsonl` line |
| hub-codegen, Rust | RETURNS-18 | `ir.json`, `wire/`, `errors.jsonl`, `aggregate.jsonl` | replaying `wire/` through `MemoryTransport` decodes the items; `errors.jsonl` yields `ClientError::Remote` of that kind; the generated `ConformanceEvent` reproduces `aggregate.jsonl` |

## Comparing

- **Hashes are real and compared exactly** (RETURNS-10; they were `"*"`
  wildcards until then). Each method's `hash` is its schema's content
  hash: drop `hash` from the method object, write the rest as canonical
  JSON (keys sorted by code point, no whitespace, arrays in order), take
  SHA-256 and keep the first 8 bytes as 16 lowercase hex. `self_hash` is
  that SHA-256/16 of the method hashes, each followed by `\n`, and `hash` of
  `self_hash` + `\n` (a hub appends its `children_hash` + `\n`).
  plexus-core's `MethodSchema::content_hash` is the reference; Python's
  `json.dumps(m, sort_keys=True, separators=(",", ":"), ensure_ascii=False)`
  writes the same bytes.
- **Metadata is kept, made deterministic.** Every frame carries
  `{"provenance":["conformance"],"plexus_hash":"60d7e85ced34c0c7","timestamp":0}`:
  `plexus_hash` is `schema.json`'s `hash`, the fixture being served as its
  own hub (a hub puts its own hash in every frame). Normalize `timestamp` to
  `0` before comparing.
- **Key order is not significant.** Compare parsed values. The server's
  content passes through `serde_json::Value`, whose keys are sorted. The
  files list keys in the types' declaration order, so a decoder that keeps
  order reproduces `aggregate.jsonl` byte for byte. Those bytes are serde's
  output for `ConformanceEvent`.

## The aggregate

`ConformanceEvent` is RETURNS-1's aggregate rule applied to this activation.
It is derived by each generator and never sent, so it is not written in
`src/`:

```rust
#[derive(Serialize, Deserialize)]
#[serde(tag = "method", content = "event", rename_all = "snake_case")]
enum ConformanceEvent {
    Ping(Pong),
    Lookup(#[serde(with = "plexus_client::wire_result")] Result<Thing, LookupError>),
    Count(Tick),
    Scan(#[serde(with = "plexus_client::wire_result")] Result<Row, ScanError>),
    Load(#[serde(with = "plexus_client::wire_result")] Result<Loaded, LoadError>),
    Warm(Warmed),
}
```

A progress update is not an item, so `load` carries `Result<Loaded,
LoadError>` and `warm` carries `Warmed`. A call failure is not an item
either.

## Where the expected files come from

They were derived before the macro could compile `src/lib.rs`: `src/`'s own
bodies, run as plain Rust, mapped to frames by RETURNS-8's frames-per-form
table, with the schema built as RETURNS-5's Intended Usage says
(`with_returns(schema_for!(T))`, …) and params as the unchanged
`{Struct}Method` enum gives them. From RETURNS-8 on, the served output is the
source: regenerate from it (`timestamp` → `0`; hashes as served since
RETURNS-10, `"*"` before), and the diff is the contract change.

RETURNS-8 served it (`tests/serve.rs`, over `Activation::call` and over the
activation's own jsonrpsee subscriptions). Every frame of every call matched
as derived. One diff, in `schema.json`: the served schema lists the
auto-generated `schema` method after the six — the macro has always put it
in every activation's schema, and the hand derivation left it out. It is now
in the file; no RETURNS-1 decision moved.

RETURNS-26 regenerated `schema.json` from the served output: the `schema`
method's `returns` went from `{}` (any value) to the schema of the
`SchemaResult` it answers, so a generated client types it. Nothing else
moved. The file keeps declaration key order: dump the served
`plugin_schema()` with `serde_json/preserve_order` on (`cargo test -p
conformance --features serde_json/preserve_order`), pretty-printed, two
spaces.

RETURNS-10 put the real hashes in: every `hash` and `self_hash` in
`schema.json`, and every frame's `plexus_hash`. So any change to the
schema — a field, a doc comment, plexus-core's own `PluginSchema` /
`MethodSchema` docs, which the `schema` method's `returns` carries — moves
the plugin hash, and with it all 25 frames' `plexus_hash`: regenerate
`schema.json` as above, then put its new `hash` in every frame. `tests/serve.rs`
(`the_served_hash_is_the_fixtures_real_hash`) and plexus-core's
`tests/conformance_fixture.rs` recompute every hash from the files.
