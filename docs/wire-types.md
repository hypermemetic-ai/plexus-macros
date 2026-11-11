# Wire types and derives

## `PlexusWire`

A type that crosses the wire is a parameter, an answer, or an item of a
stream. serde reads and writes it, and schemars describes it.
`#[derive(PlexusWire)]` is the three at once: serde's `Serialize` and
`Deserialize`, and schemars' `JsonSchema`.

<!-- tests/docs_examples.rs -->
```rust
#[derive(Debug, Clone, PartialEq, PlexusWire)]
pub struct Point {
    pub x: i32,
    #[serde(default)]
    pub y: i32,
}

#[derive(Debug, Clone, PartialEq, PlexusWire)]
pub enum CountEvent {
    Tick { n: u32 },
    AllDone,
}

#[derive(Debug, Clone, PartialEq, PlexusWire)]
#[serde(untagged)]
pub enum Either {
    Number(u32),
    Word(String),
}
```

<!-- tests/docs_examples.rs -->
```rust
assert_eq!(serde_json::to_string(&CountEvent::Tick { n: 2 }).unwrap(), r#"{"type":"tick","n":2}"#);
assert_eq!(serde_json::to_string(&CountEvent::AllDone).unwrap(), r#"{"type":"all_done"}"#);
assert_eq!(serde_json::from_str::<Point>(r#"{"x":1}"#).unwrap(), Point { x: 1, y: 0 });
assert_eq!(serde_json::to_string(&Either::Word("a".into())).unwrap(), r#""a""#);
```

- An enum is tagged `#[serde(tag = "type", rename_all = "snake_case")]`,
  unless it says its own `tag` or `untagged`.
- `#[serde(...)]` and `#[schemars(...)]` on the type, its fields and its
  variants are read as serde and schemars read them. Doc comments are in the
  schema.
- Fields may be private.
- The schema is named after the type.
- The derive is for a struct or an enum with no generic parameters. A type
  with them is a compile error that says to derive the three traits.
- The derive names `serde` and `schemars`, so the crate lists both. In a
  crate that depends on `plexus-rpc`, it names them through plexus-rpc and
  the crate lists neither.

Any type that implements the three traits is a wire type. `PlexusWire` is one
way to get them.

## `Deserialize`

`#[derive(plexus_macros::Deserialize)]` is written where serde's
`Deserialize` goes, on an internally tagged enum.

<!-- tests/docs_examples.rs -->
```rust
#[derive(Debug, Clone, PartialEq, serde::Serialize, plexus_macros::Deserialize, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TicketEvent {
    Health { version: String, count: u64 },
    Ready,
}
```

serde reads an internally tagged enum whole into a buffer, then reads the
tag, then reads the variant out of the buffer. This derive reads the first
key. When it is the tag, the variant's fields are read straight from the
rest of the object. When it is not, serde reads the object. serde writes
the tag first, so an event a service wrote is read without the buffer.

| the object | is read |
| -- | -- |
| the tag first, a struct variant | field by field, straight from the object |
| the tag first, a unit variant | the rest is read and dropped |
| the tag first, a newtype variant, a variant with a flattened field, a variant read `with` a function | by serde |
| the tag anywhere else | by serde |

Every `#[serde(...)]` attribute is read by serde's own code, and what serde
refuses is refused in serde's words. A value that only the input carries (a
`RawValue`, a map keyed by numbers) reaches a variant whose tag comes first.

The derive is for an internally tagged enum that has no generic parameters
and no untagged variant. Anything else is a compile error that says to
derive serde's `Deserialize`. The crate depends on plexus-client, which
holds the derive's runtime.

## `PlexusRequest`

`#[derive(PlexusRequest)]` is for a struct with named fields that holds what
a call's HTTP request carries: cookies, headers, the query, the peer's
address, the caller.

<!-- tests/docs_examples.rs -->
```rust
#[derive(PlexusRequest)]
pub struct AppRequest {
    /// The session cookie. Not an `Option`: a call without it is refused.
    #[from_cookie("access_token")]
    token: String,
    #[from_header("origin")]
    origin: Option<String>,
    #[from_peer]
    peer: Option<std::net::SocketAddr>,
    #[from_auth_context]
    auth: Option<plexus_core::plexus::AuthContext>,
}
```

| field attribute | the field is read from |
| -- | -- |
| `#[from_cookie("name")]` | the named cookie of the `Cookie` header |
| `#[from_header("name")]` | the named header |
| `#[from_query("name")]` | the named parameter of the URI's query. The crate depends on `form_urlencoded`. |
| `#[from_peer]` | the peer's socket address |
| `#[from_auth_context]` | the caller's `AuthContext` |
| none | the field's type, which implements `plexus_core::plexus::PlexusRequestField` |

An `Option` field is `None` when its source is absent. Any other field is
required: `extract` answers `PlexusError::Unauthenticated` when its source
is absent or does not parse into the field's type.

The derive writes `impl plexus_core::request::PlexusRequest`, with
`extract(&RawRequestContext)` and `request_schema()`, and a `JsonSchema`
impl. The schema marks each property with where it comes from:

<!-- tests/docs_examples.rs -->
```rust
let schema = AppRequest::request_schema().unwrap();
assert_eq!(schema["properties"]["token"]["x-plexus-source"], serde_json::json!({ "from": "cookie", "key": "access_token" }));
assert_eq!(schema["properties"]["origin"]["x-plexus-source"], serde_json::json!({ "from": "header", "key": "origin" }));
assert_eq!(schema["properties"]["peer"]["x-plexus-source"], serde_json::json!({ "from": "derived" }));
assert_eq!(schema["required"], serde_json::json!(["token"]));
```

### In an activation

`request = Type` on the activation names the request type. It is the
activation schema's `request`. A method takes a field of it as a parameter
marked `#[activation_param]`, with the field's name and the field's type.

<!-- tests/docs_examples.rs -->
```rust
/// An app.
#[plexus_macros::activation(request = AppRequest)]
impl App {
    /// Who the cookie says is calling.
    pub async fn me(&self, #[activation_param] token: String) -> impl Stream<Item = String> + Send + 'static {
        stream::iter([token])
    }

    /// Answers without reading the request.
    #[plexus_macros::method(request = ())]
    pub async fn health(&self) -> bool {
        true
    }
}
```

- A method with an `#[activation_param]` reads the request before its body.
  A call with no request context is refused as `Unauthenticated`. A request
  that `extract` refuses is refused as an execution error that holds
  `extract`'s message.
- A method with none reads the request too, when the call has a request
  context, and `extract`'s error refuses the call.
- `request = ()` on a method skips the read.
- `required(field, ...)` on the activation makes `request = ()` a compile
  error on its methods.
- An `#[activation_param]` whose name is no field, or whose type is not the
  field's, is a compile error. The field's type is `Clone`.
- An activation with an `#[activation_param]` method does not implement
  [its trait](activation.md#the-trait).

## `Credentials`

`#[derive(Credentials)]` marks the fields of an answer that are credentials.
Each such field carries `#[credential(...)]`, and its type is
`plexus_auth_core::Credential<T>`. Any other type is a compile error.

<!-- tests/compile/credential_basic_struct_ok.rs -->
```rust
#[derive(plexus_macros::Credentials)]
#[allow(dead_code)]
pub struct LoginEvent {
    pub user_id: String,

    #[credential(
        kind = "bearer",
        attach_as = "header:authorization",
        scheme = "Bearer ",
    )]
    pub session: Credential<SessionToken>,
}
```

<!-- tests/compile/credential_basic_struct_ok.rs -->
```rust
let entries = __plexus_credential_marker_for_LoginEvent();
assert_eq!(entries.len(), 1, "exactly one credential field");
let e = &entries[0];
assert_eq!(e.field, "session");
assert!(matches!(e.kind, CredentialKind::Bearer));
```

| clause | required | what it says |
| -- | -- | -- |
| `kind = "..."` | yes | `bearer`, `cookie`, `oauth_access`, `oauth_refresh`, `oidc_id`, `aws_sts` or `macaroon`. Another word is a compile error. |
| `kind(other = "name")` | instead of `kind = "..."` | A kind of the service's own. |
| `attach_as = "..."` | yes | Where a client puts the credential: `header:<name>`, `cookie:<name>`, `in_rpc_param:<name>` or `first_frame:<setup-method>:<param>`. |
| `scheme = "..."` | no | A prefix of the value, such as `"Bearer "`. |
| `scopes = ["a.b", ...]` | no | The scopes the credential carries. |
| `refresh_via = "ns.method"` | no | The method that refreshes it. |
| `revoke_via = "ns.method"` | no | The method that revokes it. |
| `sensitive` | no | Accepted. A credential is always sensitive. |

`expires_at` in the attribute is a compile error. The derive is for a struct
or an enum, not a union.

The derive leaves the type as it is and writes beside it:

- `__plexus_credential_marker_for_<Type>()`, one `CredentialFieldMarker` for
  each credential field, in the order they are declared;
- `__plexus_sensitive_fields_for_<Type>()`, the `(variant, field)` pairs
  that are sensitive;
- `impl HasCredentialMarkers for <Type>`, through which `#[activation]`
  finds the markers of a method's answer and puts them in the method
  schema's `credentials`, in a crate that depends on `plexus-auth-core` or
  `plexus-rpc`.

The functions name `CredentialFieldMarker`, `CredentialKind`,
`AttachmentSite`, `CredentialScheme`, `Scope`, `MethodPath`, `HeaderName`,
`CookieName`, `ParamName` and `CredentialKindName` without a path, so the
module imports them from `plexus_auth_core`.

## `HandleEnum`

`#[derive(HandleEnum)]` is for an enum of the handles an activation gives
out. A variant is a kind of handle, and its fields are the handle's `meta`.

<!-- tests/docs_examples.rs -->
```rust
pub const NOTES_PLUGIN_ID: Uuid = uuid::uuid!("550e8400-e29b-41d4-a716-446655440000");

#[derive(Debug, Clone, PartialEq, HandleEnum)]
#[handle(plugin_id = "NOTES_PLUGIN_ID", version = "1.0.0")]
pub enum NotesHandle {
    #[handle(method = "note", table = "notes", key = "id")]
    Note { note_id: String },
    #[handle(method = "message")]
    Message { message_id: String, role: String },
}
```

<!-- tests/docs_examples.rs -->
```rust
let note = NotesHandle::Note { note_id: "n-1".into() };
let handle: Handle = note.to_handle();
assert_eq!(handle.method, "note");
assert_eq!(handle.meta, ["n-1"]);
assert_eq!(NotesHandle::try_from(&handle).unwrap(), note);
assert!(note.resolution_params().is_some());
```

It writes `to_handle()`, `impl TryFrom<&Handle>`, `impl From<Enum> for Handle`
and `resolution_params()`.

| on the enum | required | what it says |
| -- | -- | -- |
| `plugin_id = "PATH"` | yes | The path of the constant that holds the activation's id. |
| `version = "..."` | yes | The version of the handles. |
| `plugin_id_type = "Type"` | no | The type the constant is an associated constant of, written in full, for an activation with generic parameters: `plugin_id = "Cone::PLUGIN_ID", plugin_id_type = "Cone<NoParent>"`. |
| `crate_path = "..."` | no | The path of plexus-core. The default is `plexus_core`. |

| on a variant | required | what it says |
| -- | -- | -- |
| `method = "..."` | yes | The handle's `method`. |
| `table = "..."`, `key = "..."` | no | The table and the key column the handle resolves in. With both, `resolution_params()` answers them for the variant. |
| `key_field = "..."` | no | The field that holds the key. The default is the first. |
| `strip_prefix = "..."` | no | A prefix taken off the key before the lookup. |

## `JsonSchema`, with the feature `schemars-compat`

With the feature `schemars-compat`, plexus-macros exports a derive named
`JsonSchema` that writes nothing. A crate that aliases plexus-macros as
`schemars` in its dev-dependencies can then write
`#[derive(PlexusRequest, schemars::JsonSchema)]` without two `JsonSchema`
impls: `PlexusRequest` writes the one.
