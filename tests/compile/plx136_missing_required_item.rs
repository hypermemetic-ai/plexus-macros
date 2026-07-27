//! PLX-136 — a missing REQUIRED item is E0046 on the generated trait impl,
//! with the whole surface enumerated. This is the error PLX-76 chose a trait
//! for.
use plexus_acp::v1::schema::{CancelNotification, NewSessionRequest, NewSessionResponse};
use plexus_acp::v1::Result;

pub struct Agent0;

#[plexus_macros::acp_agent]
impl Agent0 {
    async fn new_session(&self, _r: NewSessionRequest) -> Result<NewSessionResponse> {
        Ok(NewSessionResponse::new("s"))
    }
    async fn cancel(&self, _n: CancelNotification) -> Result<()> {
        Ok(())
    }
}

fn main() {}
