//! RED-3 compile-fail: `#[from_auth(...)]` inside an activation that
//! declares no `request = SomeType` must fail at macro expansion.

use futures::stream::Stream;
use plexus_core::plexus::{AuthContext, PlexusError};

#[derive(Clone, Debug)]
struct FakeUser;

struct BareHub;

impl BareHub {
    #[allow(dead_code)]
    async fn validate(&self, _ctx: &AuthContext) -> Result<FakeUser, PlexusError> { Ok(FakeUser) }
}

// Note: NO `request = ...` on the activation
#[plexus_macros::activation(
    namespace = "bare",
    version = "1.0.0",
    description = "RED-3 bare",
    crate_path = "plexus_core",
)]
impl BareHub {
    /// Method uses #[from_auth] but the activation has no request struct.
    #[plexus_macros::method(description = "should fail")]
    async fn sensitive(
        &self,
        #[from_auth(self.validate)] _u: FakeUser,
    ) -> impl Stream<Item = String> + Send + 'static {
        async_stream::stream! { yield "x".into(); }
    }
}

fn main() {}
