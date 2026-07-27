//! PLX-136 c2 — the strongest form: a SECOND `Agent` impl cannot coexist.
//!
//! The compile-fail fixtures next to this one close the declaration surfaces
//! `#[acp_agent]` owns — its keys, and the `initialize` body. This one closes
//! the surface it does not own: writing the trait impl yourself, by hand,
//! with a hand-built `AgentCapabilities` that says `loadSession: true` next
//! to an agent with no `load_session`.
//!
//! It is `E0119`. The generated impl is the only impl, so the derived
//! capabilities are the only capabilities — there is no second place where
//! capability truth can live, and this is the compiler saying so rather than
//! a convention anyone has to remember.
use plexus_acp::v1::schema::{
    AgentCapabilities, CancelNotification, Error, InitializeRequest, InitializeResponse,
    NewSessionRequest, NewSessionResponse, PromptRequest, PromptResponse, StopReason,
};
use plexus_acp::v1::{Agent, ProtocolVersion, Result};

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
}

impl Agent for Agent0 {
    fn initialize<'a, 'b>(
        &'a self,
        _request: InitializeRequest,
    ) -> ::core::pin::Pin<
        ::std::boxed::Box<
            dyn ::core::future::Future<Output = Result<InitializeResponse>> + Send + 'b,
        >,
    >
    where
        'a: 'b,
        Self: 'b,
    {
        Box::pin(async move {
            let mut response = InitializeResponse::new(ProtocolVersion::V1);
            let mut caps = AgentCapabilities::new();
            // The lie.
            caps.load_session = true;
            response.agent_capabilities = caps;
            Ok(response)
        })
    }

    fn new_session<'a, 'b>(
        &'a self,
        _request: NewSessionRequest,
    ) -> ::core::pin::Pin<
        ::std::boxed::Box<
            dyn ::core::future::Future<Output = Result<NewSessionResponse>> + Send + 'b,
        >,
    >
    where
        'a: 'b,
        Self: 'b,
    {
        Box::pin(async move { Ok(NewSessionResponse::new("s")) })
    }

    fn prompt<'a, 'b>(
        &'a self,
        _request: PromptRequest,
    ) -> ::core::pin::Pin<
        ::std::boxed::Box<dyn ::core::future::Future<Output = Result<PromptResponse>> + Send + 'b>,
    >
    where
        'a: 'b,
        Self: 'b,
    {
        Box::pin(async move { Ok(PromptResponse::new(StopReason::EndTurn)) })
    }

    fn cancel<'a, 'b>(
        &'a self,
        _n: CancelNotification,
    ) -> ::core::pin::Pin<
        ::std::boxed::Box<dyn ::core::future::Future<Output = Result<()>> + Send + 'b>,
    >
    where
        'a: 'b,
        Self: 'b,
    {
        Box::pin(async move { Err(Error::internal_error()) })
    }
}

fn main() {}
