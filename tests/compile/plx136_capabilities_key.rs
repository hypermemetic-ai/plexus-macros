//! PLX-136 c2 — there is no key that declares `AgentCapabilities`.
//!
//! Before `#[acp_agent]`, capability truth lived wherever an author chose to
//! write it down: a hand-built `AgentCapabilities` in `initialize` saying
//! `loadSession: true` next to an agent with no `load_session`. This fixture
//! is the shape someone reaches for first, and it does not exist.
use plexus_acp::v1::schema::{
    CancelNotification, NewSessionRequest, NewSessionResponse, PromptRequest, PromptResponse,
    StopReason,
};
use plexus_acp::v1::Result;

pub struct Agent0;

#[plexus_macros::acp_agent(capabilities = "loadSession")]
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
