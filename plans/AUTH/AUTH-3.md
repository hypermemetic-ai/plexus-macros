---
id: AUTH-3
title: "Per-method `auth = \"role:<name>\"` + claim-DSL gates"
status: Pending
type: implementation
blocked_by: [AUTH-2]
unlocks: []
---

## Problem

AUTH-2 lands activation-level `auth = "required"` — every method gets gated; presence of an `AuthContext` is required. That's a coarse gate. The hypermemetic platform — and most multi-tenant Plexus services — need finer enforcement:

1. **Role checks**: "this method is operator-only." Today a method body writes `if !auth_ctx.has_role("operator") { return Err(...) }` by hand. Forgettable. Verbose.
2. **Tenant-claim assertions**: every tenant-scoped operation must verify the caller's `claims.tenant_slug` matches the method's target tenant. This is the platform's foundational invariant per CLAUDE.md ("I use `ctx.claims.tenant_slug` for scoping — compiler/CI prevent alternatives"). Today it's a manual check, easy to forget on a new method.

This ticket adds two declarative forms on the per-method `auth = "..."` attribute (and, when sensible, on the activation-level too).

## Required behavior

### Form A — role check

```rust
#[plexus_macros::method(
    description = "List all tenants — operator only",
    auth = "role:operator",
)]
pub async fn list_all_tenants(&self) -> impl Stream<Item = Tenant> { /* auth_ctx in scope */ }
```

Codegen emits, after the existing `auth_ctx = auth.ok_or(...)?` injection:

```rust
if !auth_ctx.has_role("operator") {
    return Err(PlexusError::Forbidden(
        "method requires role 'operator'".into()
    ));
}
```

`PlexusError::Forbidden` is a new variant (or use the existing `Unauthenticated` with a distinct message — implementer's call; `Forbidden` is more typed).

Multi-role is comma-separated, OR semantics: `auth = "role:operator,admin"` accepts either role.

### Form B — claim-DSL assertion

```rust
#[plexus_macros::method(
    description = "Get a site within the caller's tenant",
    auth = "claims.tenant_slug == params.tenant",
)]
pub async fn site_get(&self, tenant: TenantSlug, name: SiteName) -> impl Stream<Item = SiteRecord> { ... }
```

The DSL is a tiny expression language with two value sources:

- `claims.<key>` — reads `auth_ctx.metadata` field by name, falls back to attempting one of the typed fields (`user_id`, `session_id`, `roles`).
- `params.<name>` — reads the named method parameter (must be a parameter that's already in scope via the macro's param-extraction; can be a `String`, `TenantSlug`, etc.).

Operators supported in V1:
- `==` — string equality (after `.to_string()` / `.as_ref()` if newtypes)
- `!=` — string inequality

V2 (separate ticket) might add `claims.X in [...]`, `claims.X starts_with "..."`, etc. V1 keeps it minimal.

Codegen for the equality check:

```rust
let claim_value: String = auth_ctx
    .get_metadata_string("tenant_slug")
    .ok_or_else(|| PlexusError::Forbidden(
        "claim 'tenant_slug' missing from auth context".into()
    ))?;
let param_value: String = tenant.to_string();   // .to_string() works for newtypes via Display
if claim_value != param_value {
    return Err(PlexusError::Forbidden(format!(
        "claim 'tenant_slug' = '{}' does not match params.tenant = '{}'",
        claim_value, param_value
    )));
}
```

Generated immediately after the auth_ctx injection, before the method body.

### Form C — combinations (later if needed)

For V1, `auth` is either a single role-form or a single DSL-form. AND-combinations like `auth = "role:operator AND claims.tenant_slug == params.tenant"` are deferred to V2 if the demand surfaces.

### Activation-level usage

`auth = "role:operator"` works at the activation level too:

```rust
#[plexus_macros::activation(
    namespace = "operator-only",
    auth = "role:operator",
    crate_path = "plexus_core",
)]
impl OperatorOnlyHub { /* every method requires the operator role */ }
```

For DSL-form (`claims.X == params.Y`), activation-level only makes sense if EVERY method has the matching `Y` parameter — otherwise codegen errors with "method `foo` doesn't have a `tenant` parameter referenced by activation-level `auth = 'claims.tenant_slug == params.tenant'`."

### Diagnostics

Compile-time:
- Unknown role syntax (`role::operator` typo) → error: "expected 'role:<name>' or 'claims.X <op> params.Y'; got 'role::operator'".
- DSL references unknown method param → error: "method `foo` declared `auth = 'claims.X == params.Y'` but has no parameter named 'Y'."
- DSL references param that isn't `Display`/`AsRef<str>` → error: "param 'X' must be `Display` to use in claim DSL."

Runtime:
- Missing claim → `PlexusError::Forbidden("claim '...' missing")`.
- Inequality → `PlexusError::Forbidden("claim '...' = '...' does not match params.X = '...'")`.

## Risks

- **DSL feature creep**. Resist. V1 is `==` / `!=` between `claims.X` and `params.Y`. Anything more is a separate ticket. The escape hatch for complex logic is: write the check by hand inside the method body using `auth_ctx`.
- **`Display` vs `AsRef<str>` requirement on params**: not every type has both. The macro tries `Display` first (works for newtypes via `#[serde(transparent)]` Display impls), falls back to `AsRef<str>` on errors. If neither fits, compile-time error tells the operator to add a `Display` impl.
- **Claim-name typos**: `claims.tenant_slugg` (typo) silently always fails Forbidden. No way to detect at compile time without typed claims. Mitigation: V2 introduces `#[derive(PlexusClaims)]` for typed claim structs; until then operators treat claim names like database column names — review carefully.
- **Order of operations relative to param extraction**: claim DSL needs the param's value, which means it must run AFTER the macro extracts params from JSON. Codegen places the check between param extraction and method body.

## What must NOT change

- AUTH-2's activation-level `auth = "required"` semantics.
- The `AuthContext` shape (no new fields needed for V1).
- The `Activation::call` trait surface.
- Today's per-method `auth: &AuthContext` parameter usage.

## Acceptance criteria

1. `auth = "role:operator"` on a method gates calls: caller without the role → `PlexusError::Forbidden(...)`; caller with the role → method body runs.
2. `auth = "role:operator,admin"` accepts either role (OR semantics).
3. `auth = "claims.tenant_slug == params.tenant"` enforces caller's claim matches the method param. Test cases: claim missing → Forbidden; claim mismatch → Forbidden; claim match → proceeds.
4. Activation-level `auth = "role:operator"` gates every method, identical to per-method usage.
5. Activation-level claim-DSL where one method lacks the referenced param → compile-time error with method name.
6. Unknown role syntax → compile-time error.
7. Macro snapshot tests covering all four forms (per-method role, activation role, per-method DSL, activation DSL).
8. Runtime test: round-trip a request through the dispatch stack, verify the right error code is emitted on each failure mode.

## Completion

`plexus-macros/src/parse.rs` extends auth-attribute parsing with role + DSL recognizers. `plexus-macros/src/codegen/activation.rs` emits the per-form check between auth_injection and the method body. New tests. CHANGELOG entry. Status flipped to Complete.

## Cross-references

- Parent epic: [AUTH-1](AUTH-1.md)
- Foundation: [AUTH-2](AUTH-2.md)
- Consumer: hypermemetic-infra `plans/AUTH/AUTH-1.md` — claim-scoped tenant operations are this consumer's foundational requirement. Their AUTH-7 ("claim-aware methods") effectively unblocks once this lands.
