//! MACRO-12 positive fixture: under `typed_params`, newtype and enum
//! parameters compile, and so does everything the server injects — `auth`,
//! `#[from_auth]`, `#[activation_param]` — even when it is a `String`. An
//! alias of `String` is not seen through (the recorded limit).

use futures::Stream;
use plexus_core::plexus::{AuthContext, PlexusError};
use plexus_macros::PlexusRequest;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct TicketId(String);

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub enum Status {
    Ready,
}

#[derive(PlexusRequest)]
pub struct Req {
    #[from_cookie("access_token")]
    pub token: String,
}

/// The known limit: an alias is not seen through.
pub type Name = String;

#[derive(Clone)]
pub struct User;

pub struct Tickets;

impl Tickets {
    async fn verify(&self, _ctx: &AuthContext) -> Result<User, PlexusError> {
        Ok(User)
    }
}

#[plexus_macros::activation(namespace = "tickets", typed_params, request = Req, crate_path = "plexus_core")]
impl Tickets {
    #[plexus_macros::method]
    async fn set_status(
        &self,
        #[from_auth(self.verify)] user: User,
        id: TicketId,
        status: Status,
        parent: Option<TicketId>,
        ids: Vec<TicketId>,
        n: Option<u32>,
        name: Name,
    ) -> impl Stream<Item = String> + Send + 'static {
        let _ = (user, id, status, parent, ids, n, name);
        futures::stream::empty()
    }

    #[plexus_macros::method]
    async fn whoami(&self, #[activation_param] token: String) -> impl Stream<Item = String> + Send + 'static {
        let _ = token;
        futures::stream::empty()
    }

    #[plexus_macros::method]
    async fn raw(&self, auth: &AuthContext) -> impl Stream<Item = String> + Send + 'static {
        let _ = auth.user_id.clone();
        futures::stream::empty()
    }
}

fn main() {}
