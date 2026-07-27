//! PLX-136 c3 — the floor case: only the four required items, and it compiles.
use plexus_acp::v1::schema::{
    CancelNotification, NewSessionRequest, NewSessionResponse, PromptRequest, PromptResponse,
    StopReason,
};
use plexus_acp::v1::{Agent, Result};

pub struct Floor;

#[plexus_macros::acp_agent(name = "floor", version = "0.1.0")]
impl Floor {
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

fn main() {
    // The trait object path ACP·E dispatches through.
    let _: Box<dyn Agent> = Box::new(Floor);
}
