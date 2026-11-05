# aggregate-tree: the RETURNS-46 fixture

The aggregate up the tree. An activation's aggregate (`{Ns}Event`, RETURNS-1)
has one variant per method carrying its item, and — since RETURNS-46 — one
per `#[child]` carrying the child's own aggregate, so the nesting goes on at
every level. A static child's variant is tagged with its route segment; a
dynamic child's with the child method's name, and carries the instance's
name beside its event.

| route | activation | namespace | aggregate | methods |
|---|---|---|---|---|
| `tree` | `TreeActivation` | `tree` | `TreeEvent` | `ping` (`-> Pong`), `count` (`-> Stream<Tick>`) |
| `tree.branch` | `BranchActivation` | `branch` | `BranchEvent` | `grow` (`-> Stream<Result<Bud, GrowError>>`) |
| `tree.branch.leaf` | `LeafActivation` | `foliage` | `FoliageEvent` | `fall` (`-> Stream<Fell>`) |
| `tree.<name>` (`a`, `b`) | `ItemActivation` | `item` | `ItemEvent` | `watch` (`-> Stream<Seen>`) |

The leaf's namespace is not its stem: its client is `LeafClient`, its
aggregate `FoliageEvent`. A parent names a child's aggregate through the
child's client (`<LeafClient<()> as plexus_client::Aggregated>::Event`), the
rule its accessor already names the client by, never by guessing.

## The wire shape

```text
{"method":"count","event":{"n":1}}                                                   a method's item
{"method":"branch","event":{"method":"grow","event":{"err":{"type":"frost","at":2}}}}  a static child's event
{"method":"branch","event":{"method":"leaf","event":{"method":"fall","event":{"leaf":1}}}}
{"method":"item","event":{"name":"a","event":{"method":"watch","event":{"seen":"a","n":1}}}}  a dynamic instance's
```

A dynamic child's content is `{"name", "event"}` under the one
`{"method", "event"}` envelope every level has — serde's adjacent tag with a
struct variant, so every generator implements the one rule RETURNS-1 set
(`tag = "method", content = "event"`), and a reader peels `method`/`event`
pairs level by level, finding a `name` only where a dynamic child is.

In Rust a dynamic child's event is boxed and its name an `Arc<str>`
(every cycle a service can serve passes through a dynamic child); a static
child's is held as is. The JSON is the same either way.

The aggregate is derived by each generator and never sent.

## Files

- `src/`: the activations. `ItemActivation`'s `watch` yields its own name,
  so a test holds each event's `name` to the instance that sent it.
- `expected/calls.json`: the calls, in order, as `{path, method, params}`;
  `path` is where the service routes them (`tree.a` for the instance `a`).
- `expected/aggregate.<level>.jsonl`: the items of the calls at or below
  `<level>`, in call order, as that level's aggregate serializes them —
  `tree` (`TreeEvent`, every call; `ping` is a one-item method, its line is
  `TreeEvent::Ping` of its answer), `tree.branch` (`BranchEvent`),
  `tree.branch.leaf` (`FoliageEvent`), `tree.item` (`ItemEvent`, the two
  instances' calls).
- `expected/ir.json`: the tree's IR 3.0 as its service writes it
  (`tree_service.ir`, plexus-core's, equal to synapse's),
  `AGGREGATE_TREE_ACCEPT=1 cargo test -p aggregate-tree --test ir` rewrites
  it. service-codegen's TypeScript aggregate is generated from a copy
  (`tests/fixtures/returns46_tree_ir.json`), with copies of the three
  static levels' JSON. The IR names the dynamic child (`tree.item`, role
  `dynamic_child`) but none of its methods: a service lists only its static
  children's schemas.

## Tests

- `tests/aggregate.rs`: each level's JSON round-trips through its aggregate
  byte for byte; wrong nestings are refused; the calls made through the
  root's client yield `aggregate.tree.jsonl` and send `calls.json`'s paths;
  `select_all` over three levels' `.events()` and two dynamic instances' is
  one `TreeEvent` stream, matched with no wildcard; a child's own aggregate
  lifts into its ancestors' by `From`.
- `tests/ir.rs`: the served IR is `expected/ir.json`.
