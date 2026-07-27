//! PLX-136 c2 — the last place a hand-written `AgentCapabilities` could live.
//!
//! Keys are not the only declaration surface: a method body that returns an
//! `InitializeResponse` is one too. `#[acp_agent]` generates `initialize`, and
//! writing one is an error, so the derived capabilities have no rival.
use plexus_acp::v1::schema::{
    AgentCapabilities, CancelNotification, InitializeRequest, InitializeResponse,
    NewSessionRequest, NewSessionResponse, PromptRequest, PromptResponse, StopReason,
};
use plexus_acp::v1::{ProtocolVersion, Result};

pub struct Agent0;

#[plexus_macros::acp_agent]
impl Agent0 {
    async fn initialize(&self, _r: InitializeRequest) -> Result<InitializeResponse> {
        let mut response = InitializeResponse::new(ProtocolVersion::V1);
        // The lie this node exists to make unrepresentable.
        let mut caps = AgentCapabilities::new();
        caps.load_session = true;
        response.agent_capabilities = caps;
        Ok(response)
    }
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
