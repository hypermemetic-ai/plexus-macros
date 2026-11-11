# Methods

A method is a function of an [activation](activation.md)'s `impl` block that
a caller reaches over the wire, as `<namespace>.<name>`.

## Which functions are methods

| the function | is |
| -- | -- |
| marked `#[plexus_macros::method]` | a method |
| `pub async fn` on `&self` returning `impl Stream<Item = ...>`, unmarked | a method |
| `pub async fn` on `&self` returning anything else, unmarked | a compile error: mark it, or make it not `pub` |
| anything else | a plain function, not served |

`methods = marked` on the activation leaves only the first row.
[`served = written`](activation.md#served--written) reads every unmarked
function from its signature. In `impl Trait for Type`, every `async fn` on
`&self` is a method.

`#[method]` is read by `#[activation]`, which removes it. `#[method]` on a
function outside an activation's block leaves the function as it is and
writes `<name>_schema()` beside it: the name, the description and the schema
of the first parameter, as JSON.

## Return forms

The return type says how many items a method answers, what an item is,
whether it can be the activation's own error, and whether progress is
reported.

| signature | items | schema `returns` / `error` / `progress` | on the wire |
| -- | -- | -- | -- |
| `-> T` | one | `T` / none / none | a data item `T` |
| `-> Result<T, E>` | one | `T` / `E` / none | a data item `{"ok": t}` or `{"err": e}` |
| `-> impl Stream<Item = T>` | many | `T` / none / none | a data item `T` each |
| `-> impl Stream<Item = Result<T, E>>` | many | `T` / `E` / none | `{"ok": t}` or `{"err": e}` each |
| `-> impl Stream<Item = Progress<T, P>>` | many | `T` / none / `P` | `Update(p)` is a progress item, `Item(t)` a data item |
| `-> impl Stream<Item = Result<Progress<T, P>, E>>` | many | `T` / `E` / `P` | `Ok(Update(p))` is a progress item, `Ok(Item(t))` is `{"ok": t}`, `Err(e)` is `{"err": e}` |

<!-- tests/docs_examples.rs -->
```rust
/// Tickets.
#[plexus_macros::activation]
impl Tickets {
    /// One item.
    #[plexus_macros::method]
    pub async fn total(&self) -> u64 {
        1
    }

    /// One item, or the activation's own error.
    #[plexus_macros::method]
    pub async fn get(&self, id: String) -> Result<Ticket, GetError> {
        if id == "T-1" { Ok(Ticket { id }) } else { Err(GetError::NotFound { id }) }
    }

    /// Many items.
    pub async fn list(&self) -> impl Stream<Item = Ticket> + Send + 'static {
        stream::iter([Ticket { id: "T-1".into() }])
    }

    /// Many items, each of which can be an error. An `Err` does not end the stream.
    pub async fn check(&self, ids: Vec<String>) -> impl Stream<Item = Result<Ticket, GetError>> + Send + 'static {
        stream::iter(ids).map(|id| if id == "T-1" { Ok(Ticket { id }) } else { Err(GetError::NotFound { id }) })
    }

    /// Many items, with progress of the default type, `Note`.
    pub async fn export(&self) -> impl Stream<Item = Progress<Ticket>> + Send + 'static {
        stream::iter([
            Progress::Update(Note { message: "reading".into(), percentage: Some(50.0) }),
            Progress::Item(Ticket { id: "T-1".into() }),
        ])
    }

    /// Many items, with progress of a declared type, and errors.
    pub async fn import(&self, source: String) -> impl Stream<Item = Result<Progress<Ticket, ImportStep>, GetError>> + Send + 'static {
        stream::iter([
            Ok(Progress::Update(ImportStep::Chunk { done: 1, of: 1 })),
            Ok(Progress::Item(Ticket { id: source })),
        ])
    }
}
```

- An `Err` is data: the activation's answer, not the call failing. A stream
  goes on past it unless the method ends it. A call that fails (a bad
  parameter, an unknown method, a caller refused) is an error item, which
  ends the call.
- `Progress<T>` reports `Note { message, percentage }`. `Progress<T, P>`
  reports `P`. `Progress` and `Note` are `plexus_core::plexus::{Progress, Note}`,
  the same types as `plexus_client::{Progress, Note}`.
- Progress needs a stream. `-> Progress<T>` is a compile error.
- The forms are read from the tokens: a type whose last path segment is
  `Result` with two type arguments, or `Progress` with one or two. An alias
  of either (`anyhow::Result<T>`) is a plain `T`.
- `impl Stream<Item = ...>` takes any further bounds, `+ Send + 'static`
  among them.
- Any other return type is a compile error that lists the six forms. No
  return type is one. So are `BoxStream<'_, T>`, `Pin<Box<dyn Stream<...>>>`
  and `PlexusStream`: return `impl Stream<Item = ...>`.

## Parameters

The parameters after `&self` are the call's parameters, passed by name in a
JSON object. Each type is read with serde and described with schemars. An
`Option<T>` parameter is not required.

Each parameter is read by itself, so a refused call names its parameter:
`invalid_params` with the parameter and `missing`, or `malformed` and a
detail.

<!-- tests/docs_examples.rs -->
```rust
let notes = Untyped::new(InProcessTransport::new(Notes), "notes");
let refused = notes.call_one("search", json!({})).await.unwrap_err();
assert!(matches!(
    refused,
    ClientError::Remote(RemoteError::InvalidParams { param: Some(param), problem: ParamProblem::Missing }) if param == "text"
));
```

Some parameters are not the caller's. The server passes them, and the
client's method does not take them.

| parameter | what it is |
| -- | -- |
| `auth: &AuthContext`, `auth: Option<&AuthContext>` | The caller. See [Callers](callers.md). |
| `reach: Reach` | Where the caller holds the method's grant. First, or right after `auth`. |
| `#[from_auth(expr)] name: T` | A value a resolver makes from the caller. |
| `#[activation_param] name: T` | A field of the activation's [request type](wire-types.md#plexusrequest). |
| `name: Input<T>` | A stream of items from the caller. See [below](#input). |
| `parent: Parent<Client>` | The service the activation is [grafted onto](children.md#an-activation-that-names-its-parent). After the others of this table. |

`auth`, `reach`, an `Input` and `parent` are not in the schema's `params`. A
`#[from_auth]` value is a property there, marked
`"x-plexus-source": { "from": "auth", "resolver": "..." }` and not required.
The fields of the activation's request type are properties there too, each
marked with its own `x-plexus-source`, in a method that has parameters of
its own and does not say `request = ()`.

A parameter is described by a doc comment before it, or by `params(...)` on
the method, which wins. Without either its description is
`The <name> parameter`.

## `#[method(...)]`

<!-- tests/docs_examples.rs -->
```rust
/// Find notes.
#[plexus_macros::method(
    name = "search",
    group = "read",
    http_method = "GET",
    params(limit = "At most this many"),
    values(status = ["open", "closed"]),
    example(note = "the first ten open notes", params = r#"{"text": "plexus", "status": "open", "limit": 10}"#),
)]
pub async fn find(
    &self,
    /** Words to look for */ text: String,
    status: Option<String>,
    limit: Option<u32>,
) -> Vec<String> {
```

| argument | what it does |
| -- | -- |
| `name = "..."` | The method's name on the wire. The default is the function's name. |
| `description = "..."` | Replaces the `///` comment, which is then the schema's `long_description`. |
| `params(name = "...", ...)` | Parameter descriptions. |
| `group = "..."` | The method's group in a listing: `MethodSchema.group`. One per method. |
| `http_method = "GET"` | `GET`, `POST`, `PUT`, `DELETE` or `PATCH`, for a client that speaks HTTP. The schema says `POST` without it. |
| `values(param = ["A", "B"], ...)` | The values a parameter is meant to take: `x-plexus-values` on its schema. A hint, not checked at a call. |
| `example(note = "...", params = r#"{...}"#)` | A worked call: `MethodSchema.examples`. Repeatable. The params are a JSON object. |
| `request = ()` | The method does not read the activation's [request type](wire-types.md#plexusrequest). |
| `public`, `everywhere`, `target = "param"`, `scope = "..."` | Who may call. See [Callers](callers.md). |
| `answer_by = "caller" \| "person" \| "grant:<activation.method>"` | `MethodSchema.answer_by`. `caller` is the default and is left out of the schema. |
| `input_window = N`, `input_max_items = N`, `input_max_bytes = N`, `input_max_buffered = N` | Limits on the method's `Input`. See [below](#input). |
| `uses("shop.items", ...)` | Methods of the [parent](children.md#an-activation-that-names-its-parent) the method calls. |

`values(...)`, `example(...)` and `target` name the method's own parameters.
A name that is not one is a compile error that lists them. So is a word
`#[method]` does not take.

## `Input`

A parameter of type `Input<T>` (`plexus_core::plexus::Input`) is a stream of
`T` that the caller sends while the call runs. A method takes at most one. It
is a parameter of the method itself, not a part of another type. The schema
carries `T` as the method's `input`, and the client's method takes a stream.

<!-- tests/docs_examples.rs -->
```rust
/// Count a run's tokens as they arrive.
#[plexus_macros::method(input_window = 256)]
pub async fn count(&self, run: String, tokens: Input<String>) -> u32 {
    let _ = run;
    tokens.into_stream().count().await as u32
}
```

The parameters are read before the input opens. `input_window` is the
credit the call grants its caller. `input_max_buffered` is the bytes held
past which it grants no more until they are read. `input_max_items` and
`input_max_bytes` limit the whole call. Without them the limits are those of
plexus-core's `CallCeiling::default()`. Any of the four on a method with no
`Input` is a compile error, and so is a window of 0.

## `typed_params`

With `typed_params` on the activation:

- A parameter written `String`, `&str`, `Option<String>`, `Option<&str>` or
  `Vec<String>` is a compile error at the parameter. The parameters the
  server passes are exempt. A type alias of `String` is not seen.
- A call whose params object has a key that is no parameter is refused
  before the method runs, as `invalid_params`, in the words
  ``unknown field `limt`; ledger.entries takes: limit``. The message names
  every unknown key and every parameter of the method.

<!-- tests/docs_examples.rs -->
```rust
let ledger = Untyped::new(InProcessTransport::new(Ledger), "ledger");
assert_eq!(ledger.call_one("entries", json!({ "limit": 1 })).await.unwrap(), json!([0]));
let refused = ledger.call_one("entries", json!({ "limt": 1 })).await.unwrap_err();
assert!(matches!(refused, ClientError::Remote(RemoteError::InvalidParams { .. })), "{refused:?}");
```

Without `typed_params`, a key that is no parameter is ignored.
