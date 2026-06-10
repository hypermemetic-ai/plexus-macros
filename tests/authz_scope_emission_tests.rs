//! R-1 acceptance tests (ROLES+AP merged wave; trak facet
//! `977c04ed-684c-4833-abef-5e9411c7de45`): the macro emits
//! `scope = "..."` / `public` / declared auth posture into the generated
//! `MethodSchema` entries via plexus-core's builders.
//!
//! Coverage (AUTHZ-CORE-2 acceptance criteria + AP-1 criteria):
//!
//! | Case | Verified by |
//! |---|---|
//! | single `scope = "..."` → populated `requires_credential` | `single_scope_populates_requires_credential` |
//! | multi `scope = ...` → conjunction (ALL scopes) | `multi_scope_emits_conjunction` |
//! | `public` → `MethodSchema.public = true` | `public_method_emits_public_flag` |
//! | no annotation → wire identical to pre-R-1 (regression) | `unannotated_method_wire_shape_unchanged` |
//! | declared posture stamped on every RPC method (AP-1 #1) | `declared_posture_stamped_on_methods` |
//! | undeclared posture emits nothing (AP-1 #2) | `unannotated_method_wire_shape_unchanged` |
//! | explicit `auth = "optional"` still emitted (declared ≠ default) | `explicit_optional_posture_is_emitted` |
//! | posture + scope + public combine on one activation | `posture_scope_and_public_combine` |
//! | schema JSON fixture: scope= shows in `requires_credential` | `single_scope_populates_requires_credential` (JSON half) |
//!
//! Compile-fail cases (`public` + `scope` contradiction, invalid scope
//! literal) are trybuild fixtures in `tests/compile/authz_*.rs`, driven by
//! `tests/authz_scope_compile_tests.rs`.
//!
//! Run with: `cargo test --test authz_scope_emission_tests`.

use async_stream::stream;
use futures::stream::Stream;
use plexus_core::plexus::schema::Scope;
use plexus_core::plexus::{AuthPosture, AuthContext, PlexusError, RequiredCredential};
use plexus_macros::PlexusRequest;

// ---------------------------------------------------------------------------
// Fixture: explicit scopes + public + unannotated, no posture declared.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct ScopedFixture;

#[plexus_macros::activation(
    namespace = "scoped_fixture",
    version = "1.0.0",
    crate_path = "plexus_core"
)]
impl ScopedFixture {
    /// Requires exactly one explicit scope.
    #[plexus_macros::method(scope = "fixture.write")]
    async fn write(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "w".into(); }
    }

    /// Requires BOTH listed scopes (conjunction).
    #[plexus_macros::method(scope = "fixture.write", scope = "fixture.admin")]
    async fn admin_write(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "aw".into(); }
    }

    /// Explicitly public — exempt from the default-deny gate.
    #[plexus_macros::method(public)]
    async fn ping(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "pong".into(); }
    }

    /// No authz annotation — implicit `<activation>.<method>` rule applies
    /// at registry-build time; the IR carries nothing.
    #[plexus_macros::method]
    async fn plain(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "p".into(); }
    }
}

fn find(schemas: &[plexus_core::plexus::MethodSchema], name: &str) -> plexus_core::plexus::MethodSchema {
    schemas
        .iter()
        .find(|m| m.name == name)
        .unwrap_or_else(|| panic!("method `{name}` missing from schemas"))
        .clone()
}

/// AUTHZ-CORE-2: one `scope = "..."` → `requires_credential` populated via
/// `RequiredCredential::from_method_scope`. Also the R-1/R-2 verification
/// fixture: the method's schema JSON shows the populated field.
#[test]
fn single_scope_populates_requires_credential() {
    let schemas = ScopedFixtureMethod::method_schemas();
    let write = find(&schemas, "write");

    let expected = RequiredCredential::from_method_scope(Scope::new("fixture.write"));
    assert_eq!(write.requires_credential.as_ref(), Some(&expected));

    // The wire JSON half: schema JSON for the annotated method carries the
    // populated requires_credential.
    let json = serde_json::to_value(&write).expect("schema serializes");
    assert_eq!(
        json["requires_credential"]["scopes"],
        serde_json::json!(["fixture.write"]),
        "scope= must surface in the schema JSON, got {json}"
    );
    assert!(
        json["requires_credential"].get("kind").is_none(),
        "kind must stay None (omitted) for scope-derived requirements"
    );
}

/// AUTHZ-CORE-2: repeated `scope = ...` → conjunction, order preserved.
#[test]
fn multi_scope_emits_conjunction() {
    let schemas = ScopedFixtureMethod::method_schemas();
    let admin_write = find(&schemas, "admin_write");

    let req = admin_write
        .requires_credential
        .as_ref()
        .expect("multi-scope method must carry requires_credential");
    assert_eq!(
        req.scopes,
        vec![Scope::new("fixture.write"), Scope::new("fixture.admin")],
        "ALL declared scopes must be carried, in declaration order"
    );
    assert!(req.kind.is_none());
    assert!(req.site_hint.is_none());
}

/// AUTHZ-CORE-2: `public` → `MethodSchema.public = true`, on the wire, with
/// no `requires_credential`.
#[test]
fn public_method_emits_public_flag() {
    let schemas = ScopedFixtureMethod::method_schemas();
    let ping = find(&schemas, "ping");

    assert!(ping.public, "public flag must be set on the schema");
    assert!(ping.requires_credential.is_none());

    let json = serde_json::to_value(&ping).expect("schema serializes");
    assert_eq!(json["public"], true);
}

/// Regression (AUTHZ-CORE-2 AC #7 + AP-1 AC #2): an unannotated method on a
/// posture-silent activation has wire JSON with NONE of the R-1 fields —
/// byte-identical to pre-R-1 emission.
#[test]
fn unannotated_method_wire_shape_unchanged() {
    let schemas = ScopedFixtureMethod::method_schemas();
    let plain = find(&schemas, "plain");

    assert!(plain.requires_credential.is_none());
    assert!(!plain.public);
    assert!(plain.auth_posture.is_none());

    let json = serde_json::to_value(&plain).expect("schema serializes");
    let obj = json.as_object().unwrap();
    assert!(!obj.contains_key("requires_credential"), "got {json}");
    assert!(!obj.contains_key("public"), "got {json}");
    assert!(!obj.contains_key("auth_posture"), "got {json}");
}

// ---------------------------------------------------------------------------
// Fixture: declared posture (no auth machinery needed for "mixed").
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct MixedPosture;

#[plexus_macros::activation(
    namespace = "mixed_posture",
    version = "1.0.0",
    auth = "mixed",
    crate_path = "plexus_core"
)]
impl MixedPosture {
    #[plexus_macros::method]
    async fn alpha(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "a".into(); }
    }

    #[plexus_macros::method]
    async fn beta(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "b".into(); }
    }
}

/// AP-1 AC #1: a declared posture produces IR carrying `auth_posture` with
/// the declared value — on every RPC method of the activation.
#[test]
fn declared_posture_stamped_on_methods() {
    let schemas = MixedPostureMethod::method_schemas();
    for name in ["alpha", "beta"] {
        let m = find(&schemas, name);
        assert_eq!(
            m.auth_posture,
            Some(AuthPosture::Mixed),
            "declared posture must be stamped on `{name}`"
        );
        let json = serde_json::to_value(&m).unwrap();
        assert_eq!(json["auth_posture"], "mixed");
    }

    // The framework-injected auto `schema` method is NOT stamped — the
    // declaration describes the activation's RPC enforcement surface.
    let auto_schema = find(&schemas, "schema");
    assert!(auto_schema.auth_posture.is_none());
}

// ---------------------------------------------------------------------------
// Fixture: explicit `auth = "optional"` — declared ≠ silently defaulted.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct ExplicitOptional;

#[plexus_macros::activation(
    namespace = "explicit_optional",
    version = "1.0.0",
    auth = "optional",
    crate_path = "plexus_core"
)]
impl ExplicitOptional {
    #[plexus_macros::method]
    async fn gamma(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "g".into(); }
    }
}

/// An EXPLICIT `auth = "optional"` is a declaration and gets emitted, even
/// though enforcement-wise it matches the undeclared default.
#[test]
fn explicit_optional_posture_is_emitted() {
    let schemas = ExplicitOptionalMethod::method_schemas();
    let gamma = find(&schemas, "gamma");
    assert_eq!(gamma.auth_posture, Some(AuthPosture::Optional));
}

// ---------------------------------------------------------------------------
// Fixture: posture + scope + public combined on one activation
// (`auth = "required"`: every method is gated or public).
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct FakeUser;

#[derive(PlexusRequest)]
struct Req {
    #[from_cookie("access_token")]
    #[allow(dead_code)]
    auth_token: String,
}

#[derive(Clone)]
struct RequiredCombo;

impl RequiredCombo {
    #[allow(dead_code)]
    async fn validate(&self, _: &AuthContext) -> Result<FakeUser, PlexusError> {
        Ok(FakeUser)
    }
}

#[plexus_macros::activation(
    namespace = "required_combo",
    version = "1.0.0",
    request = Req,
    auth = "required",
    crate_path = "plexus_core"
)]
impl RequiredCombo {
    /// Auth-gated AND scope-annotated.
    #[plexus_macros::method(scope = "combo.write")]
    async fn write(
        &self,
        #[from_auth(self.validate)] _u: FakeUser,
    ) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "w".into(); }
    }

    /// Public escape hatch under `auth = "required"`.
    #[plexus_macros::method(public)]
    async fn login(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "ok".into(); }
    }
}

/// Posture, scope, and public all carry simultaneously: the gated method has
/// posture + requires_credential; the public method has posture + public and
/// no requires_credential.
#[test]
fn posture_scope_and_public_combine() {
    let schemas = RequiredComboMethod::method_schemas();

    let write = find(&schemas, "write");
    assert_eq!(write.auth_posture, Some(AuthPosture::Required));
    assert_eq!(
        write.requires_credential.as_ref(),
        Some(&RequiredCredential::from_method_scope(Scope::new(
            "combo.write"
        )))
    );
    assert!(!write.public);

    let login = find(&schemas, "login");
    assert_eq!(login.auth_posture, Some(AuthPosture::Required));
    assert!(login.public);
    assert!(login.requires_credential.is_none());

    let json = serde_json::to_value(&login).unwrap();
    assert_eq!(json["auth_posture"], "required");
    assert_eq!(json["public"], true);
}
