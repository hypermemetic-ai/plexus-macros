---
id: AUTH-2
title: "Activation-level `auth = \"required\"` attribute"
status: Pending
type: implementation
blocked_by: []
unlocks: [AUTH-3]
---

## Problem

`#[plexus_macros::method]` already supports per-method auth gating: when a method has `auth: &AuthContext` in its signature, codegen at `crates/plexus-macros/src/codegen/activation.rs:908` emits:

```rust
let auth_ctx = auth.ok_or_else(|| PlexusError::Unauthenticated(
    "This method requires authentication".to_string()
))?;
```

…right before the method body runs. So unauth calls to that method get a typed Unauthenticated error.

But the activation-level attribute (`#[plexus_macros::activation(...)]`) — which today accepts `namespace`, `version`, `description`, `crate_path`, `request`, `resolve_handle`, `hub`, `children`, `plugin_id`, `namespace_fn` — has **no `auth` key**. To make every method on a namespace require auth today you must either:

1. Add `auth: &AuthContext` to every method signature individually (forgetting one is silent),
2. Declare `request = MyRequest` where `MyRequest` has a `#[from_auth]` field (abuses the request-extraction path — every method must take the request struct, even ones that don't care about its other fields).

Both are foot-guns. This ticket adds activation-level gating as a first-class attribute.

## Required behavior

### Syntax

```rust
#[plexus_macros::activation(
    namespace = "whoami",
    auth = "required",
    crate_path = "plexus_core",
)]
impl WhoamiHub {
    #[plexus_macros::method(description = "Returns the caller's identity")]
    pub async fn whoami(&self) -> impl Stream<Item = Identity> { /* auth_ctx exists in scope */ }

    #[plexus_macros::method(description = "Echoes a message")]
    pub async fn echo(&self, message: String) -> impl Stream<Item = String> { /* auth_ctx in scope */ }
}
```

When `auth = "required"` is present at the activation level:

1. **Every method** on the activation gets the same auth-injection prelude that per-method `auth: &AuthContext` would produce: `let auth_ctx = auth.ok_or(PlexusError::Unauthenticated(...))?;`
2. **`auth_ctx` is in scope inside the method body** as `&AuthContext`, even though the method's signature doesn't declare it. (The macro inserts the local binding.)
3. **A method with its own `auth: &AuthContext` parameter** alongside activation-level `auth = "required"` is allowed but redundant — the macro emits a compile-time `note` (not error) recommending the operator delete the per-method parameter.

### Override / opt-out

A method can opt OUT of activation-level auth with `#[method(auth = "public")]`:

```rust
#[plexus_macros::activation(namespace = "whoami", auth = "required", crate_path = "plexus_core")]
impl WhoamiHub {
    #[plexus_macros::method(description = "Heartbeat", auth = "public")]
    pub async fn health(&self) -> impl Stream<Item = Health> { /* runs without auth */ }
}
```

This is the explicit "I know what I'm doing" escape hatch for health/readiness endpoints. Without it, `health` would also require auth.

### Accepted values for `auth = ...`

V1 accepts:
- `"required"` — auth-injection emitted; method body has `auth_ctx` available
- `"public"` (per-method only) — opt out from activation-level requirement

V2 (AUTH-3) extends with `"role:operator"`, `"claims.tenant_slug == params.tenant"`, etc. V1 should parse these gracefully without erroring — accept them, ignore the value beyond `required`, emit a `compile_warning` saying "auth = 'role:X' not yet implemented; treating as 'required' for V1."

### Codegen changes

`plexus-macros/src/parse.rs` — extend `HubMethodsAttrs` parser (line 290+) with an `auth: Option<String>` field. Parse `auth = "required"` / `auth = "public"` like the existing string-valued attrs (description, namespace, etc.).

`plexus-macros/src/codegen/activation.rs` — when `HubMethodsAttrs.auth == Some("required")` AND the method's own `auth` value isn't `"public"`, set `requires_auth = true` for that method (regardless of whether the signature has `auth: &AuthContext`). The existing auth_injection block (line 908) handles the rest unchanged.

If the method's signature DOES contain `auth: &AuthContext`, the macro should still consume that parameter normally (don't double-inject `auth_ctx`).

## Risks

- **Method signatures without `auth: &AuthContext` parameter and activation-level `auth = "required"`**: today's per-method check trades the typed parameter for compile-time visibility — you can grep `auth: &AuthContext` to find authenticated methods. With activation-level gating, the auth-context is implicit. Mitigation: a generated module-level constant or a `#[doc]` comment on the activation impl block listing which methods are auth-gated, so `cargo doc` and IDE tooling surface it.
- **Mixing per-method `auth: &AuthContext` with activation-level `auth = "required"`**: existing methods that already declare the parameter shouldn't break. The macro keeps consuming the parameter; just doesn't double-emit the injection.
- **`"public"` as a magic string**: typos like `auth = "publoc"` would silently keep the method auth-gated. Mitigation: V1 errors at compile time on any unrecognized value other than `"required"` / `"public"` (V2 will accept additional patterns).

## What must NOT change

- Existing per-method `auth: &AuthContext` works exactly as today. Activation-level gating is **additive**.
- Existing `#[from_auth(...)]` resolvers still trigger `requires_auth = true` per-method.
- The `Activation::call` trait surface (`auth: Option<&AuthContext>`, `raw_ctx: Option<&RawRequestContext>`). No changes.
- The `AuthContext` shape itself.

## Acceptance criteria

1. `#[plexus_macros::activation(namespace = "x", auth = "required", crate_path = "...")]` compiles and gates every `#[method]` on the impl block.
2. A test calling such a method without auth → `PlexusError::Unauthenticated`.
3. A test calling the same method with `auth = Some(&AuthContext { ... })` → method body runs; `auth_ctx` is in scope.
4. `#[method(description = "...", auth = "public")]` opts a single method out of the activation-level requirement; calls without auth succeed.
5. A method with both `auth: &AuthContext` parameter and activation-level `auth = "required"` compiles without double-injection (warns, doesn't error).
6. Unknown `auth = "..."` value at the activation level → compile-time error: `auth attribute supports "required"; got "<value>". Future ticket AUTH-3 will add role + claim DSL.`
7. Macro snapshot / golden test in `plexus-macros/tests/` covering the codegen output for: (a) activation-level required, no per-method override; (b) activation-level required + per-method public; (c) per-method only (today's behavior — unchanged).

## Completion

`plexus-macros/src/parse.rs` extends `HubMethodsAttrs` with `auth: Option<String>`. `plexus-macros/src/codegen/activation.rs` propagates the activation-level flag into per-method `requires_auth`. Tests added per ACs. CHANGELOG entry. Status flipped to Complete.

## Cross-references

- Parent epic: [AUTH-1](AUTH-1.md)
- First known consumer: hypermemetic-infra's `plans/AUTH/AUTH-1.md` and `plans/PLATFORM/PLATFORM-44.md` (in their repo, not this one).
