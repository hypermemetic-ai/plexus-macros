//! MACRO-13: a contract's methods are what its implementor is called with,
//! so one that takes what the server injects is named.

use futures::Stream;
use plexus_core::plexus::AuthContext;

#[plexus_macros::activation]
impl GuardActivation {
    pub async fn greet(&self, auth: &AuthContext, greeting: String) -> impl Stream<Item = String> + Send + 'static;
}

fn main() {}
