//! PLX-136 — a misspelled item is E0407, not a silently-inert method.
//!
//! `ext(acp_role = "promt")` would have expanded clean and failed at runtime.
//! Here `promt` is a method not in the ACP vocabulary, and the compiler says so.
use plexus_acp::v1::schema::{
    CancelNotification, NewSessionRequest, NewSessionResponse, PromptRequest, PromptResponse,
    StopReason,
};
use plexus_acp::v1::Result;

pub struct Agent0;

#[plexus_macros::acp_agent]
impl Agent0 {
    async fn new_session(&self, _r: NewSessionRequest) -> Result<NewSessionResponse> {
        Ok(NewSessionResponse::new("s"))
    }
    async fn promt(&self, _r: PromptRequest) -> Result<PromptResponse> {
        Ok(PromptResponse::new(StopReason::EndTurn))
    }
    async fn cancel(&self, _n: CancelNotification) -> Result<()> {
        Ok(())
    }
}

fn main() {}
