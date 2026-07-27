//! PLX-136, advertisement site 3 — implementing `authenticate` makes
//! `acp_auth_methods` a COMPILE-TIME obligation.
//!
//! ACP gates `authenticate` on `InitializeResponse.authMethods` being
//! non-empty. This crate cannot invent auth methods, so the generated
//! `initialize` reaches for the author's — and only when `authenticate` is
//! implemented, which is why the obligation cannot be advertised without the
//! method that services it, or implemented without being advertised.
use plexus_acp::v1::schema::{
    AuthenticateRequest, AuthenticateResponse, CancelNotification, NewSessionRequest,
    NewSessionResponse, PromptRequest, PromptResponse, StopReason,
};
use plexus_acp::v1::Result;

pub struct Agent0;

#[plexus_macros::acp_agent]
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
    async fn authenticate(&self, _r: AuthenticateRequest) -> Result<AuthenticateResponse> {
        Ok(AuthenticateResponse::new())
    }
}

fn main() {}
