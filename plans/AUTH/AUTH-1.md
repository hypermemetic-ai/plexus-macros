---
id: AUTH-1
title: "AUTH — Auth-aware codegen in plexus-macros"
status: Epic
type: epic
blocked_by: []
unlocks: []
external_consumers:
  - hypermemetic-infra:plans/AUTH/AUTH-1.md  # the IdP+JWT+SecretStore epic that depends on this
  - hypermemetic-infra:plans/PLATFORM/PLATFORM-44.md  # wire-SessionValidator-everywhere (filed separately)
---

## Goal

`plexus-macros` already supports per-method auth gating: when a method's signature contains `auth: &AuthContext` or a `#[from_auth(resolver)]` parameter, the generated dispatch wraps the call with `let auth_ctx = auth.ok_or(PlexusError::Unauthenticated(...))?;`. That's good — it works.

What's missing:

1. **Activation-level gating.** `#[plexus_macros::activation(...)]` accepts `namespace`, `version`, `description`, `crate_path`, `request`, `resolve_handle`, `hub`, `children` — but no `auth = "required"`. To gate every method on a namespace today you must either (a) annotate every method individually (forgetting one is a silent unauthenticated method) or (b) declare a `request = MyRequest` where `MyRequest` has a `#[from_auth]` field — a workaround that abuses the request-extraction path for an unrelated purpose.

2. **Declarative role checks.** Today every method body that wants to enforce a role (`admin`, `operator`, etc.) writes the check by hand: `if !auth_ctx.has_role("operator") { return Err(...) }`. A declarative `auth = "role:operator"` on the method (or activation) is the obvious shape.

3. **Claim-scoped tenant assertions.** The hypermemetic platform's whole authorization model rides on `claims.tenant_slug` matching the operation's target tenant. CLAUDE.md says: *"I use `ctx.claims.tenant_slug` for scoping — compiler/CI prevent alternatives."* Today this is a runtime check the implementer must remember to write per-method. A declarative `auth = "claims.tenant_slug == params.tenant"` would push it to compile/dispatch time.

4. **Claim extraction shortcut.** `#[from_auth(|auth| async { ... })]` works but is verbose for the common case of "pull a metadata field by name." `#[from_claims("tenant_id")]` would be the shorthand.

## Supports

- **hypermemetic-infra `AUTH-1`** (the epic at `plans/AUTH/AUTH-1.md` in that repo): the platform validates JWTs and propagates `claims.tenant_slug` everywhere. Without activation/method-level gating + claim-scoping in plexus-macros, the platform implements every check by hand, in every method body. The macro layer is where this should live.
- **hypermemetic-infra `PLATFORM-44`** (filed separately in that repo, blocked on this epic): wire `SessionValidator` into every plexus hub the platform deploys (platform itself, whoami, runs, future backends). With this epic landed, those hubs declare `auth = "required"` once at the activation level and every method is automatically gated.
- Any other Plexus-based service that wants principled auth without re-implementing the same checks per method body.

## User stories

**As a backend author**

```rust
#[plexus_macros::activation(
    namespace = "whoami",
    auth = "required",                       // AUTH-2: gate every method
    crate_path = "plexus_core",
)]
impl WhoamiHub {
    #[plexus_macros::method(description = "Return the caller's identity")]
    pub async fn whoami(
        &self,
        #[from_claims("tenant_id")] tenant_id: String,   // AUTH-4: claim shortcut
    ) -> impl Stream<Item = Identity> { ... }

    #[plexus_macros::method(
        description = "Operator-only — list all tenants",
        auth = "role:operator",                          // AUTH-3: role check
    )]
    pub async fn list_tenants(&self) -> impl Stream<Item = Tenant> { ... }

    #[plexus_macros::method(
        description = "Tenant-scoped read",
        auth = "claims.tenant_slug == params.tenant",   // AUTH-3: claim DSL
    )]
    pub async fn get_thing(&self, tenant: TenantSlug) -> impl Stream<Item = Thing> { ... }
}
```

**As an integrator**: I see `auth = "required"` in the activation attribute and know every method on the namespace is auth-gated. No need to grep for missing per-method annotations.

**As a security reviewer**: I can audit one site (the activation attribute) instead of N (one per method). Forgetting to annotate a new method is impossible — the activation says "all of them."

## Dependency DAG

```
plexus-core 0.5.x AuthContext + RawRequestContext  (already shipped)
                            │
                            ▼
              AUTH-2 (activation-level required)
                            │
                            ├──────────────┐
                            ▼              ▼
              AUTH-3 (per-method      AUTH-4 (#[from_claims]
              auth = "role:..."       extractor shorthand)
              + claims DSL)
```

AUTH-2 is the foundation. AUTH-3 + AUTH-4 are independent extensions on top.

## Tickets

| ID | Type | Summary | Status |
|---|---|---|---|
| AUTH-2 | implementation | Activation-level `auth = "required"` attribute | Pending |
| AUTH-3 | implementation | Per-method `auth = "role:<name>"` + claim-DSL gates | Pending |
| AUTH-4 | implementation | `#[from_claims("key")]` parameter extractor | Pending |

## Out of scope

- The `SessionValidator` impl itself (Logto / Keycloak / CF Access JWT validation) — that's a backend concern, not a macro concern. plexus-macros stays IdP-agnostic; it just operates on the `AuthContext` that the validator produces.
- AuthContext schema changes (adding fields beyond `user_id` / `session_id` / `roles` / `metadata`). The current shape is sufficient for everything in this epic.
- Forwarding `RawRequestContext` between hubs — already supported by `Activation::call`'s `raw_ctx: Option<&RawRequestContext>` parameter. This epic is about making the **method-side** claim checks declarative; the forwarding side is unchanged.
- Multi-IdP composition (one hub federating multiple SessionValidators). Single-IdP-per-hub is the V1 assumption; multi-IdP is a follow-up epic if it ever becomes a real need.

## Consumer reference

The hypermemetic-infra repo's `plans/AUTH/AUTH-1.md` is the **first known consumer** of this epic. That epic delegates to:
- An IdP (Logto Cloud, in their case) for JWT issuance.
- A `SessionValidator` impl that turns a JWT cookie into `AuthContext`.
- The plexus-macros codegen (this epic) for per-call enforcement.

When that AUTH-1 lands its first method-level claim assertion, this epic's AUTH-3 should be promoted to Ready first so the implementation has the macro support to consume.
