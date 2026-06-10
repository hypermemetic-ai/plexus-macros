---
id: AUTH-4
title: "`#[from_claims(\"key\")]` parameter extractor shorthand"
status: Pending
type: implementation
blocked_by: [AUTH-2]
unlocks: []
---

## Problem

`#[from_auth(...)]` exists today: it takes a closure that runs against `&AuthContext` and binds the result as a typed local. Useful when the resolution is non-trivial (DB lookup, etc.). But the **common case** is "pull the named field from `auth_ctx.metadata` (a `serde_json::Value`) and parse it as a String / typed newtype." For that, the closure boilerplate is overkill:

```rust
// Today's verbose form:
#[from_auth(|auth| async move {
    auth.get_metadata_string("tenant_id")
        .ok_or_else(|| PlexusError::Forbidden("missing tenant_id claim".into()))
})] tenant_id: String
```

Every method that needs the tenant id from the JWT writes that closure. It's the kind of thing macros are for.

This ticket adds `#[from_claims("key")]` as a sibling shorthand to `#[from_auth(...)]`, parallel to how `#[from_cookie("name")]`, `#[from_header("name")]`, and `#[from_peer]` all exist as typed extractors on `PlexusRequest`.

## Required behavior

### Syntax

```rust
#[plexus_macros::method(description = "Get tenant info from JWT claims")]
pub async fn whoami_tenant(
    &self,
    #[from_claims("tenant_id")] tenant_id: String,
    #[from_claims("email")] email: Option<String>,            // Option = optional claim
    #[from_claims("preferred_username")] user: TenantSlug,    // newtype with FromStr impl
) -> impl Stream<Item = Identity> { ... }
```

### Codegen

For `#[from_claims("key")] name: T`:

- If `T = String`: emit
  ```rust
  let name: String = auth_ctx
      .get_metadata_string("key")
      .ok_or_else(|| PlexusError::Forbidden(
          "claim 'key' missing from auth context".into()
      ))?;
  ```
- If `T = Option<String>`: emit
  ```rust
  let name: Option<String> = auth_ctx.get_metadata_string("key");
  ```
- If `T` impls `FromStr` (newtype like `TenantSlug`): emit the String extraction + `T::from_str(&s).map_err(|e| PlexusError::Forbidden(format!("claim 'key' invalid: {}", e)))?`.
- If `T = i64` / `u64` / `bool`: pull from `auth_ctx.metadata.get("key").and_then(|v| v.as_i64()/as_u64()/as_bool())` with appropriate error.

The method's signature parameter is consumed (don't include in JSON-RPC params extraction). Same pattern as `#[from_auth(...)]`.

### Implies `requires_auth = true`

Like `#[from_auth(...)]`, presence of `#[from_claims(...)]` on any param sets the method's `requires_auth = true`. The macro emits the standard auth-injection prelude (`let auth_ctx = auth.ok_or(...)?`) before extracting the claim.

### Composes with AUTH-3 DSL

```rust
#[plexus_macros::method(
    description = "Site read, scoped to caller's tenant",
    auth = "claims.tenant_slug == params.tenant",
)]
pub async fn site_get(
    &self,
    tenant: TenantSlug,    // from JSON params
    #[from_claims("tenant_slug")] caller_tenant: TenantSlug,   // from JWT
) -> impl Stream<Item = SiteRecord> { ... }
```

The DSL check at AUTH-3 still runs (against `params.tenant`); the `caller_tenant` param is available inside the method body as a typed value, useful for logging or further checks.

### Naming consistency

The framework already has:
- `#[from_cookie("name")]`
- `#[from_header("name")]`
- `#[from_query("name")]` (if it exists; check the codebase)
- `#[from_peer]`
- `#[from_auth(closure)]`

`#[from_claims("name")]` slots in cleanly.

### Diagnostics

Compile-time:
- Unsupported type T (no String/Option<String>/FromStr/numeric/bool path) → error: "type T not supported by #[from_claims]; supported: String, Option<String>, T: FromStr, i64, u64, bool. Use #[from_auth(...)] for custom resolution."
- Empty key → error: "from_claims requires a non-empty key string."

Runtime: see codegen above; missing claim → Forbidden, parse failure → Forbidden with reason.

## Risks

- **Implicit dependency on `auth_ctx.metadata` shape**: the `metadata` field is a `serde_json::Value`. Operators must know what keys their IdP populates. Mitigation: V2 (later ticket) introduces `#[derive(PlexusClaims)]` for typed claim structs; this ticket is the simple-stringly-typed shorthand.
- **Type-coercion edge cases**: a JWT might emit `tenant_id` as a number while the macro expects a string (or vice versa). V1 strict-types: `#[from_claims("tenant_id")] x: String` requires the claim to be JSON-string. Numeric claims need `#[from_claims("...")] x: i64`. No silent type coercion.
- **Conflict with `#[from_auth(...)]`** on the same parameter: should error.

## What must NOT change

- `#[from_auth(...)]` semantics (still the escape hatch for custom resolution).
- `AuthContext` shape.
- The `requires_auth` flag mechanics — `from_claims` triggers it the same way `from_auth` does.

## Acceptance criteria

1. `#[from_claims("k")] x: String` extracts `auth_ctx.metadata["k"]` as String; missing → Forbidden.
2. `#[from_claims("k")] x: Option<String>` returns None when missing instead of erroring.
3. `#[from_claims("k")] x: TenantSlug` (newtype with FromStr) extracts + parses; parse error → Forbidden.
4. `#[from_claims("k")] x: i64` extracts as JSON number.
5. Unsupported type → compile-time error naming the type and the supported list.
6. Same parameter with both `#[from_auth(...)]` and `#[from_claims(...)]` → compile-time error.
7. Macro snapshot tests covering all extraction shapes.

## Completion

`plexus-macros/src/parse.rs` extends parameter parsing with `from_claims` recognition. `plexus-macros/src/codegen/activation.rs` emits the typed extraction code per-supported-type. CHANGELOG entry. Status flipped to Complete.

## Cross-references

- Parent epic: [AUTH-1](AUTH-1.md)
- Foundation: [AUTH-2](AUTH-2.md)
- Sibling: [AUTH-3](AUTH-3.md) — DSL `claims.X == params.Y` enforcement; this ticket gives operators typed access to those same claims.
- Consumer: hypermemetic-infra `plans/AUTH/AUTH-1.md` — every tenant-scoped method needs the caller's `tenant_slug` from JWT.
