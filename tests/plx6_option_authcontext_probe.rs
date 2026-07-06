//! PLX-6 probe: does the 0.6 `#[plexus_macros::activation]` macro SUPPORT a
//! handler declaring an OPTIONAL auth param (`auth: Option<&AuthContext>`) so a
//! public-read method runs WITHOUT an authenticated caller — alongside a
//! required-`&AuthContext` handler?
//!
//! Prior audit facet (trak S3) claimed the macro FORCES auth even for
//! `Option<&AuthContext>`, blocking public-read. This test decides it:
//!   - it must COMPILE (macro accepts the optional shape), and
//!   - the optional-auth handler must return Ok when called with `auth = None`
//!     and observe `None`, while the required-auth handler must still 401.
//!
//! Idiom mirrored from tests/optional_auth_runtime_tests.rs.

use async_stream::stream;
use futures::stream::Stream;
use futures::StreamExt;
use plexus_core::plexus::{Activation, AuthContext, PlexusError};

struct Probe;

#[plexus_macros::activation(
    namespace = "plx6_probe",
    version = "1.0.0",
    auth = "mixed",
    crate_path = "plexus_core",
)]
impl Probe {
    /// Public-read: optional auth. Must run with no authenticated caller.
    #[plexus_macros::method(public)]
    async fn read(
        &self,
        auth: Option<&AuthContext>,
    ) -> impl Stream<Item = String> + Send + 'static {
        let label = match auth {
            Some(ctx) => format!("some:{}", ctx.user_id),
            None => "none".to_string(),
        };
        stream! { yield label; }
    }

    /// Required auth: must 401 when no AuthContext is attached.
    #[plexus_macros::method]
    async fn write(
        &self,
        auth: &AuthContext,
    ) -> impl Stream<Item = String> + Send + 'static {
        let id = auth.user_id.clone();
        stream! { yield id; }
    }
}

async fn first_string(mut s: plexus_core::plexus::PlexusStream) -> Option<String> {
    while let Some(item) = s.next().await {
        if let plexus_core::plexus::PlexusStreamItem::Data { content, .. } = item {
            if let serde_json::Value::String(v) = content {
                return Some(v);
            }
        }
    }
    None
}

#[tokio::test]
async fn optional_auth_public_read_runs_without_caller() {
    let p = Probe;
    // No auth attached (None) — S3 says this should be forced/blocked.
    let stream = p
        .call("read", serde_json::json!({}), None, None)
        .await
        .expect("PLX-6: public-read Option<&AuthContext> must accept None auth");
    assert_eq!(first_string(stream).await.as_deref(), Some("none"));
}

#[tokio::test]
async fn required_auth_still_rejects_missing_caller() {
    let p = Probe;
    // PlexusStream isn't Debug (boxed trait object), so match without {:?}.
    match p.call("write", serde_json::json!({}), None, None).await {
        Err(PlexusError::Unauthenticated(_)) => {}
        Err(other) => panic!("required &AuthContext handler: expected Unauthenticated, got {other:?}"),
        Ok(_) => panic!("required &AuthContext handler must 401 on missing auth; got Ok"),
    }
}
