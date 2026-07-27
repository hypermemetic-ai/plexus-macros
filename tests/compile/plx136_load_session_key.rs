//! PLX-136 c2 — nor one that declares a single capability by its own name.
use plexus_acp::v1::schema::{
    CancelNotification, NewSessionRequest, NewSessionResponse, PromptRequest, PromptResponse,
    StopReason,
};
use plexus_acp::v1::Result;

pub struct Agent0;

#[plexus_macros::acp_agent(load_session = "true")]
impl Agent0 {
    async fn new_session(&self, _r: NewSessionRequest) -> Result<NewSessionResponse> {
        Ok(NewSessionResponse::new("s"))
    }
    async fn prompt(&self, _r: PromptRequest) -> Result<PromptResponse> {
        Ok(PromptResponse::new(StopReason::EndTurn))
    }
    async fn cancel(&self, _n: CancelNotification) -> Result<()> {
        Ok(())
    }
}

fn main() {}
