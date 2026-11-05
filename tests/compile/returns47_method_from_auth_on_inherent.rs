//! RETURNS-47: `#[from_auth(…)]` on a method is the form for `impl Trait for
//! Type`; on an inherent method it goes on the parameter.

use plexus_core::plexus::{AuthContext, PlexusError};

#[derive(Clone)]
pub struct Notes;

impl Notes {
    async fn verify(&self, _ctx: &AuthContext) -> Result<String, PlexusError> {
        Ok(String::new())
    }
}

#[plexus_macros::activation]
impl Notes {
    #[plexus_macros::from_auth(self.verify)]
    pub async fn get(&self) -> impl futures::Stream<Item = String> + Send + 'static {
        futures::stream::empty()
    }
}

fn main() {}
