//! PLX-136 c2, advertisement site 2 — modes are advertised per session, on
//! the `NewSessionResponse`, and there is no initialize-time key for them.
//! This is the pair PLX-135 warned would be dropped.
use plexus_acp::v1::schema::{
    CancelNotification, NewSessionRequest, NewSessionResponse, PromptRequest, PromptResponse,
    StopReason,
};
use plexus_acp::v1::Result;

pub struct Agent0;

#[plexus_macros::acp_agent(modes = "plan,build")]
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
