# Callers and roles

The macros do two things about who is calling. They hand a method its
caller, and they put in the schema what a service needs to decide a call:
the method's grant, its target, whether it is public, and the activation's
roles. The decision is made by the service's gate, in plexus-core.

<!-- tests/docs_examples.rs -->
```rust
/// A help desk.
#[plexus_macros::activation(
    request = DeskRequest,
    auth = "mixed",
    roles(
        viewer = ["get", "mine"],
        editor = [viewer, "close"],
    ),
)]
impl Desk {
    /// Open to everyone.
    #[plexus_macros::method(public)]
    pub async fn hours(&self) -> String {
        "9 to 5".into()
    }

    /// One ticket. The gate decides the call where `id` is.
    #[plexus_macros::method(target = "id")]
    pub async fn get(&self, id: String) -> String {
        id
    }

    /// The caller, required: an anonymous call is refused before the body.
    #[plexus_macros::method]
    pub async fn whoami(&self, auth: &AuthContext) -> String {
        auth.user_id.clone()
    }

    /// The caller, when there is one.
    #[plexus_macros::method]
    pub async fn greeting(&self, auth: Option<&AuthContext>) -> String {
        match auth {
            Some(auth) => format!("hello, {}", auth.user_id),
            None => "hello".into(),
        }
    }

    /// A value made from the caller by a resolver.
    #[plexus_macros::method]
    pub async fn close(&self, #[from_auth(self.member)] member: Member, id: String) -> String {
        format!("{} closed {id}", member.id)
    }

    /// Where the caller holds this method's grant.
    pub async fn mine(&self, reach: Reach) -> impl Stream<Item = String> + Send + 'static {
        stream::iter(reach.held_on().iter().map(|scope| scope.to_string()).collect::<Vec<_>>())
    }

    /// Acts on the whole service: admitted only where its grant is held on `*`.
    #[plexus_macros::method(everywhere)]
    pub async fn purge(&self) -> u32 {
        0
    }

    /// Needs both scopes, where the service has a scope registry.
    #[plexus_macros::method(scope = "desk.export", scope = "desk.read")]
    pub async fn export(&self) -> Vec<String> {
        Vec::new()
    }
}
```

## The caller as a parameter

| parameter | the method gets | with no caller |
| -- | -- | -- |
| `auth: &AuthContext` | the caller | the call is refused as `unauthenticated`, before the body |
| `auth: Option<&AuthContext>` | the caller, or `None` | the body runs with `None` |
| `#[from_auth(expr)] name: T` | what `expr(&auth).await` answers `Ok` | the call is refused as `unauthenticated`, before the resolver |
| `reach: Reach` | where the caller holds the method's grant, as the gate answered | see [below](#reach) |

The parameter is named `auth` and its type ends in `AuthContext`
(`plexus_core::plexus::AuthContext`). None of these is a parameter of the
call. See [Methods](methods.md#parameters).

<!-- tests/docs_examples.rs -->
```rust
let desk = Untyped::new(InProcessTransport::new(Desk), "desk");
let refused = desk.call_one("whoami", serde_json::json!({})).await.unwrap_err();
assert!(matches!(refused, ClientError::Remote(RemoteError::Unauthenticated)), "{refused:?}");
let refused = desk.call_one("close", serde_json::json!({ "id": "T-1" })).await.unwrap_err();
assert!(matches!(refused, ClientError::Remote(RemoteError::Unauthenticated)), "{refused:?}");
assert_eq!(desk.call_one("greeting", serde_json::json!({})).await.unwrap(), "hello");
assert_eq!(desk.call_one("hours", serde_json::json!({})).await.unwrap(), "9 to 5");
```

A method with none of these parameters runs for every call that reaches it.
Which calls reach it is [the gate's](#what-the-gate-enforces).

### `#[from_auth]`

The resolver is an expression that is called with `&AuthContext` and
awaited, such as `self.member` for
`async fn member(&self, auth: &AuthContext) -> Result<Member, PlexusError>`.
Its `Ok` value is the parameter.

Its error refuses the call. A `PlexusError` reaches the caller as it is, so
a resolver's `Forbidden` or `Unauthenticated` stays typed. Any other error
that implements `Display` is an execution error holding its text.

The macro runs the resolver and does not judge it. A resolver that answers
`Ok` for a caller lets that caller in.

`#[from_auth]` is on a parameter, in an activation that names a
`request = Type`. Without the request type it is a compile error. In
`impl Trait for Type` it is on the method:
[`#[plexus_macros::from_auth(expr)]`](activation.md#impl-trait-for-type).

### `reach`

`reach: Reach` (`plexus_core::plexus::Reach`) is the gate's answer for this
call: the scopes on which the caller holds the method's grant. A method uses
it to answer only what its caller reaches. In a service with no gate the
reach is everywhere. Outside a dispatched call there is no reach, and the
method is refused as `unauthenticated`. `reach` is the first parameter, or
the one right after `auth`.

## The `auth` argument

`auth = "..."` on the activation is checked when the crate compiles.

| `auth = ...` | what the macro checks |
| -- | -- |
| `"optional"`, or nothing | Nothing is refused. When some methods have a `#[from_auth]` parameter and others have neither one nor `public`, the build warns and names them. |
| `"mixed"` | Nothing is refused, and there is no warning. |
| `"required"` | Every method has a `#[from_auth]` parameter or is `public`. Any other method is a compile error. A method that takes `auth: Option<&AuthContext>` is a warning instead. |
| `"none"` | No method has a `#[from_auth]` parameter. One that does is a compile error. |

A posture that is written is in every method's schema, as `auth_posture`.
Another word is a compile error.

In `impl Trait for Type`, `"required"` refuses a caller who is not signed in
before the body of every method that is not `public`.

## What the schema says

| written | in the schema | meaning |
| -- | -- | -- |
| nothing | `MethodSchema.grant` is `<namespace>.<method>` | The grant a caller holds to call the method. |
| `#[method(public)]` | `MethodSchema.public` | Anyone may call it. |
| `#[method(target = "id")]` | `MethodSchema.target` | The parameter that says what the call acts on. The gate decides the call on the scope that value belongs to. |
| `#[method(everywhere)]` | `MethodSchema.everywhere` | The method acts on the whole service. Its grant counts only where it is held on `*`. |
| `#[method(scope = "a.b")]` | `MethodSchema.requires_credential` | A scope the caller must have. Repeated, the caller must have all of them. |
| `roles(...)` on the activation | `PluginSchema.roles` | The roles the activation declares, each a list of grants. |

- `target` names one of the method's own parameters. Another name is a
  compile error that lists them.
- A scope is dot-separated segments of `[a-z][a-z0-9_]*`, with an optional
  last segment `*`, or `*` alone. Another shape is a compile error.
- `public` beside `scope`, `public` beside `everywhere` and `everywhere`
  beside `target` are compile errors.
- A method marked `everywhere` also checks, before its body, that the call
  was admitted on `*`, and refuses it as `forbidden` otherwise. A call made
  outside any dispatch is not checked.

<!-- tests/docs_examples.rs -->
```rust
assert!(method("hours").public);
assert_eq!(method("get").target.as_deref(), Some("id"));
assert_eq!(method("get").grant.as_deref(), Some("desk.get"));
assert!(method("purge").everywhere);
assert!(method("export").requires_credential.is_some());

assert_eq!(schema.roles["viewer"], ["desk.get", "desk.mine"]);
assert_eq!(schema.roles["editor"], ["desk.get", "desk.mine", "desk.close"]);
```

## Roles

`roles(name = [entries], ...)` declares the activation's roles. An entry is:

- a method of the activation, as a string: `"get"` is the grant `desk.get`;
- a role declared before it, as a bare word: `viewer` is every grant of
  `viewer`;
- a method of a dynamic child, as `"{child}.method"`, where `child` is the
  function that answers the dynamic child: the grant
  `<namespace>.{child}.method`.

A method that is not the activation's, a role that is not declared earlier
and a role declared twice are compile errors. `admin` is built in and holds
every grant. Declaring it, or listing it in another role, is a compile
error.

The roles are data in the schema. A service gathers the roles of every
activation it serves, merged by name, with `DynamicHub::declared_roles()`,
and seeds its gate's role table with them.

## What the gate enforces

The macro writes none of these decisions. plexus-core makes them before it
routes a call, from the schema the macro wrote.

**A service with a grant gate** (`DynamicHub::with_gate`) denies by default:

| the call | a caller who is not signed in | a signed-in caller |
| -- | -- | -- |
| a `public` method | runs | runs |
| `<namespace>.schema` | runs | runs |
| a method with a `target`, and the value present | `unauthenticated` | runs when a role the caller holds, on a scope the value belongs to, lists the method's grant |
| a method marked `everywhere` | `unauthenticated` | runs when a role the caller holds on `*` lists the grant |
| any other method | `unauthenticated` | runs when a role the caller holds anywhere lists the grant |

A refusal for a missing grant is `forbidden`, and names the grant and the
scope. A session's ceiling narrows every row. A child's method is decided by
its whole path (`desk.archive.get`), and a static child's roles are gathered
under that path.

**A service with a scope registry** (`DynamicHub::with_scope_registry`)
checks a method's `scope = "..."` scopes: a caller who is not signed in is
`unauthenticated`, and a signed-in caller who lacks one is `forbidden`,
naming it. A `public` method passes.

**A service with neither** routes every call. A method is then guarded only
by its own parameters: `auth: &AuthContext` and `#[from_auth]` refuse a call
with no caller, and a resolver refuses whom it chooses to.
