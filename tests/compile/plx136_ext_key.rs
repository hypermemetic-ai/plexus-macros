//! PLX-136, required behaviour 4 — `ext(...)` does not survive as a mapping
//! mechanism. PLX-76 rejected `ext(acp_role = "prompt")` because a
//! misspelling expands fine and fails at runtime against a live editor; the
//! trait is the mapping, and ACP's own extension channel is the reserved
//! underscore-prefixed method namespace.
use plexus_acp::v1::schema::{
    CancelNotification, NewSessionRequest, NewSessionResponse, PromptRequest, PromptResponse,
    StopReason,
};
use plexus_acp::v1::Result;

pub struct Agent0;

#[plexus_macros::acp_agent(ext = "acp_role")]
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
