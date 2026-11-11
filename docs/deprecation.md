# Deprecation

## A deprecated method, child, parameter or activation

Rust's `#[deprecated]` on a method puts the deprecation in the method's
schema. `#[plexus_macros::removed_in("...")]` beside it says the version the
method goes away in, which `#[deprecated]` has no key for.

<!-- tests/docs_examples.rs -->
```rust
/// A clock.
#[plexus_macros::activation]
impl Clock {
    /// The time, in seconds.
    #[deprecated(since = "0.5.0", note = "use `now` instead")]
    #[plexus_macros::removed_in("0.6.0")]
    #[plexus_macros::method]
    pub async fn time(&self) -> u64 {
        0
    }

    /// The time, in milliseconds.
    #[plexus_macros::method]
    pub async fn now(
        &self,
        #[deprecated(since = "0.5.0", note = "the zone is always UTC")] zone: Option<String>,
    ) -> u64 {
        let _ = zone;
        0
    }
}
```

<!-- tests/docs_examples.rs -->
```rust
assert_eq!(
    method("time")["deprecation"],
    serde_json::json!({ "since": "0.5.0", "removed_in": "0.6.0", "message": "use `now` instead" })
);
let zone = method("now")["params_meta"].as_array().unwrap().iter().find(|p| p["name"] == "zone").unwrap().clone();
assert_eq!(zone["deprecation"]["removed_in"], "unspecified");
```

| `#[deprecated]` on | is in the schema as |
| -- | -- |
| a method | `MethodSchema.deprecation` |
| a `#[child]` function | the child's `MethodSchema.deprecation` |
| a parameter | `deprecation` of the parameter's entry in `MethodSchema.params_meta` |
| the `impl` block | `PluginSchema.deprecation` |

`deprecation` holds `since`, `removed_in` and `message`: the attribute's
`since`, the companion's version, and the attribute's `note`. Without
`#[removed_in]`, `removed_in` is `"unspecified"`. `since` and `message` are
empty strings when the attribute does not give them.

`#[removed_in]` without `#[deprecated]` on the same item is a compile error.
Its argument is one string.

rustc warns wherever a deprecated function is called, and the code the
macro writes calls every method. `#[allow(deprecated)]` on the module that
holds the activation silences that warning.

## Also accepted, with a deprecation warning

Each of these compiles, and the build warns at it.

| written | what it does | written without a warning |
| -- | -- | -- |
| `#[plexus_macros::hub_methods]` | The same as `#[activation]`. | `#[plexus_macros::activation]` |
| `#[plexus_macros::hub_method]` | The same as `#[method]`. | `#[plexus_macros::method]` |
| `client` on `#[activation]` | Nothing: the client is written whenever plexus-client is a dependency. | leave it out |
| `impl Foo` with `namespace = "..."`, where `Foo` does not end in `Activation` and no `trait` is named | Only functions marked `#[method]` are methods, the client is written only with `client`, and no trait is written. `server_cfg` there is a compile error without `client`. | leave `namespace` out, or name the type `FooActivation` |
| `hub` on `#[activation]` | No `ChildRouter` impl is written: the type writes its own, and the schema's children are those `self.plugin_children()` answers. A compile error beside `#[child]`. | `#[child]` functions |
| `children = [field, ...]` on `#[activation]` | Each named field of the type is a child. A compile error beside `#[child]`. | `#[child]` functions |
| `streaming` on `#[method]` | Nothing, on a method that returns a stream. A compile error on a method that answers one item. | leave it out |
| `#[child(list = "names")]`, `#[child(search = "find")]`, the function as a string | The same as the bare name. | `list = names`, `search = find` |
