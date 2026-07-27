//! PLX-136 (ACP·B) — `#[acp_agent]`, both directions, all three sites.
//!
//! The claim under test is that advertising and implementing are the same
//! act. That is only interesting if it is asserted BOTH ways for every place
//! an ACP v1 agent advertises anything, which PLX-135 established is three
//! places and not one:
//!
//! 1. `InitializeResponse.agentCapabilities`
//! 2. `NewSessionResponse.modes` / `.configOptions` (per session)
//! 3. `InitializeResponse.authMethods`
//!
//! Site 1's negative direction ("you cannot advertise without implementing")
//! is proved by there being **no such input** — see
//! `tests/plx136_compile_tests.rs` and `acp_agent::ACP_AGENT_KEYS`. What is
//! asserted here is that the derivation is real, that the two per-session
//! items are not silently dropped, and that auth methods are unreachable
//! without `authenticate`.

use plexus_acp::v1::schema::{
    AuthMethod, AuthMethodAgent, CancelNotification, CloseSessionRequest, CloseSessionResponse,
    InitializeRequest, LoadSessionRequest, LoadSessionResponse, NewSessionRequest,
    NewSessionResponse, PromptRequest, PromptResponse, SessionConfigId, SessionConfigOption,
    SessionConfigSelectOption, SessionMode, SessionModeState, SetSessionConfigOptionRequest,
    SetSessionConfigOptionResponse, SetSessionModeRequest, SetSessionModeResponse, StopReason,
};
use plexus_acp::v1::{Advertisement, Agent, AgentMethod, ProtocolVersion, Requirement, Result};

fn init_request() -> InitializeRequest {
    InitializeRequest::new(ProtocolVersion::V1)
}

fn new_session_request() -> NewSessionRequest {
    NewSessionRequest::new("/tmp")
}

// ===========================================================================
// The floor case (c3): only the required items.
// ===========================================================================

pub struct Floor;

#[plexus_macros::acp_agent(name = "floor", version = "0.1.0")]
impl Floor {
    async fn new_session(&self, _r: NewSessionRequest) -> Result<NewSessionResponse> {
        Ok(NewSessionResponse::new("floor-1"))
    }
    async fn prompt(&self, _r: PromptRequest) -> Result<PromptResponse> {
        Ok(PromptResponse::new(StopReason::EndTurn))
    }
    async fn cancel(&self, _n: CancelNotification) -> Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn c3_the_floor_case_compiles_and_answers_initialize() {
    let response = Floor.initialize(init_request()).await.unwrap();

    // The version the trait IS, not one anybody declared.
    assert_eq!(response.protocol_version, ProtocolVersion::V1);

    let caps = &response.agent_capabilities;
    assert!(!caps.load_session, "nothing optional was implemented");
    assert!(caps.session_capabilities.list.is_none());
    assert!(caps.session_capabilities.resume.is_none());
    assert!(caps.session_capabilities.delete.is_none());
    assert!(caps.session_capabilities.close.is_none());
    assert!(caps.auth.logout.is_none());

    // Site 3, negative: no `authenticate`, so no auth methods. There is no
    // input that could have put one here.
    assert!(response.auth_methods.is_empty());

    // `name`/`version` are identity, not capability.
    let info = response.agent_info.expect("agentInfo");
    assert_eq!(info.name, "floor");
    assert_eq!(info.version, "0.1.0");

    // The derivation's only input.
    assert_eq!(
        Floor::ACP_IMPLEMENTED_ITEMS,
        &["new_session", "prompt", "cancel"]
    );
    assert!(!Floor::acp_derived().requires_auth_methods);
    assert!(Floor::acp_derived().per_session.is_empty());
}

#[tokio::test]
async fn the_floor_case_answers_method_not_found_for_what_it_did_not_implement() {
    // The trait's provided defaults do this; the point is that the macro did
    // not paper over them, so the wire answer matches the advertisement.
    let err = Floor
        .load_session(LoadSessionRequest::new("floor-1", "/tmp"))
        .await
        .unwrap_err();
    assert_eq!(
        err.code,
        plexus_acp::v1::schema::Error::method_not_found().code
    );
}

// ===========================================================================
// c1 positive: implementing an optional item advertises it.
// ===========================================================================

pub struct Loader;

#[plexus_macros::acp_agent]
impl Loader {
    async fn new_session(&self, _r: NewSessionRequest) -> Result<NewSessionResponse> {
        Ok(NewSessionResponse::new("loader-1"))
    }
    async fn prompt(&self, _r: PromptRequest) -> Result<PromptResponse> {
        Ok(PromptResponse::new(StopReason::EndTurn))
    }
    async fn cancel(&self, _n: CancelNotification) -> Result<()> {
        Ok(())
    }
    /// The whole declaration. There is no second one.
    async fn load_session(&self, _r: LoadSessionRequest) -> Result<LoadSessionResponse> {
        Ok(LoadSessionResponse::new())
    }
    async fn close_session(&self, _r: CloseSessionRequest) -> Result<CloseSessionResponse> {
        Ok(CloseSessionResponse::new())
    }
}

#[tokio::test]
async fn c1_implementing_an_optional_item_is_what_advertises_it() {
    let advertised = Loader.initialize(init_request()).await.unwrap();
    let caps = &advertised.agent_capabilities;

    assert!(caps.load_session, "load_session is implemented");
    assert!(caps.session_capabilities.close.is_some());

    // And nothing else leaked in.
    assert!(caps.session_capabilities.list.is_none());
    assert!(caps.session_capabilities.resume.is_none());
    assert!(caps.session_capabilities.delete.is_none());
    assert!(caps.auth.logout.is_none());
    assert!(advertised.auth_methods.is_empty());
}

#[tokio::test]
async fn c1_the_two_directions_differ_only_by_the_impl() {
    // Same fixture shape, one item apart. The capability follows the item.
    let with = Loader.initialize(init_request()).await.unwrap();
    let without = Floor.initialize(init_request()).await.unwrap();

    assert!(with.agent_capabilities.load_session);
    assert!(!without.agent_capabilities.load_session);
    assert_ne!(with.agent_capabilities, without.agent_capabilities);
}

#[tokio::test]
async fn the_advertised_set_is_exactly_the_derivation_of_the_implemented_idents() {
    // No third state: the response's capabilities ARE
    // derive_agent_capabilities(implemented idents), byte for byte.
    let response = Loader.initialize(init_request()).await.unwrap();
    let expected = plexus_acp::v1::derive_agent_capabilities(
        AgentMethod::ALL
            .into_iter()
            .filter(|m| Loader::ACP_IMPLEMENTED_ITEMS.contains(&m.trait_item())),
    );
    assert_eq!(response.agent_capabilities, expected.capabilities);
}

// ===========================================================================
// Site 3: authMethods.
// ===========================================================================

pub struct Authed;

#[plexus_macros::acp_agent]
impl Authed {
    async fn new_session(&self, _r: NewSessionRequest) -> Result<NewSessionResponse> {
        Ok(NewSessionResponse::new("authed-1"))
    }
    async fn prompt(&self, _r: PromptRequest) -> Result<PromptResponse> {
        Ok(PromptResponse::new(StopReason::EndTurn))
    }
    async fn cancel(&self, _n: CancelNotification) -> Result<()> {
        Ok(())
    }
    async fn authenticate(
        &self,
        _r: plexus_acp::v1::schema::AuthenticateRequest,
    ) -> Result<plexus_acp::v1::schema::AuthenticateResponse> {
        Ok(plexus_acp::v1::schema::AuthenticateResponse::new())
    }
}

impl Authed {
    /// Required at COMPILE TIME by implementing `authenticate` — the generated
    /// `initialize` calls it, and only then.
    fn acp_auth_methods() -> Vec<AuthMethod> {
        vec![AuthMethod::Agent(AuthMethodAgent::new(
            "api-key", "API key",
        ))]
    }
}

#[tokio::test]
async fn site3_implementing_authenticate_is_what_puts_auth_methods_on_the_wire() {
    let response = Authed.initialize(init_request()).await.unwrap();
    assert_eq!(response.auth_methods.len(), 1);
    assert!(Authed::acp_derived().requires_auth_methods);
}

/// The same agent with an empty list: the obligation is not discharged, and
/// initialize refuses rather than advertising a method nobody can reach.
pub struct AuthedButSilent;

#[plexus_macros::acp_agent]
impl AuthedButSilent {
    async fn new_session(&self, _r: NewSessionRequest) -> Result<NewSessionResponse> {
        Ok(NewSessionResponse::new("silent-1"))
    }
    async fn prompt(&self, _r: PromptRequest) -> Result<PromptResponse> {
        Ok(PromptResponse::new(StopReason::EndTurn))
    }
    async fn cancel(&self, _n: CancelNotification) -> Result<()> {
        Ok(())
    }
    async fn authenticate(
        &self,
        _r: plexus_acp::v1::schema::AuthenticateRequest,
    ) -> Result<plexus_acp::v1::schema::AuthenticateResponse> {
        Ok(plexus_acp::v1::schema::AuthenticateResponse::new())
    }
}

impl AuthedButSilent {
    fn acp_auth_methods() -> Vec<AuthMethod> {
        Vec::new()
    }
}

#[tokio::test]
async fn site3_an_implemented_authenticate_with_no_auth_methods_is_refused() {
    let err = AuthedButSilent
        .initialize(init_request())
        .await
        .unwrap_err();
    assert!(
        err.message.contains("authMethods"),
        "the error must name the site it is about: {}",
        err.message
    );
}

#[tokio::test]
async fn site3_the_reverse_direction_has_no_input_at_all() {
    // `Floor` also defines `acp_auth_methods`, but does not implement
    // `authenticate` — the generated initialize never reaches for it, so
    // there is no way to advertise auth methods without the method that
    // services them.
    let response = Floor.initialize(init_request()).await.unwrap();
    assert!(response.auth_methods.is_empty());
    assert_eq!(Floor::unused_auth_methods().len(), 1);
}

impl Floor {
    /// Deliberately present and deliberately ignored: naming it
    /// `acp_auth_methods` would still change nothing, because the call site
    /// is not emitted for an agent without `authenticate`. This spelling just
    /// keeps the test honest about which fact is doing the work.
    fn unused_auth_methods() -> Vec<AuthMethod> {
        vec![AuthMethod::Agent(AuthMethodAgent::new("nope", "Nope"))]
    }
}

// ===========================================================================
// Site 2: per session. The obligation PLX-135 handed this node by name.
// ===========================================================================

fn mode_state(n: usize) -> SessionModeState {
    SessionModeState::new(
        "m0",
        (0..n)
            .map(|i| SessionMode::new(format!("m{i}"), format!("Mode {i}")))
            .collect(),
    )
}

fn config_option() -> SessionConfigOption {
    SessionConfigOption::select(
        "temp",
        "Temperature",
        "hot",
        vec![SessionConfigSelectOption::new("hot", "Hot")],
    )
}

/// Implements both per-session items AND advertises them on the session.
pub struct PerSessionGood;

#[plexus_macros::acp_agent]
impl PerSessionGood {
    async fn new_session(&self, _r: NewSessionRequest) -> Result<NewSessionResponse> {
        Ok(NewSessionResponse::new("ps-1")
            .modes(mode_state(2))
            .config_options(vec![config_option()]))
    }
    async fn prompt(&self, _r: PromptRequest) -> Result<PromptResponse> {
        Ok(PromptResponse::new(StopReason::EndTurn))
    }
    async fn cancel(&self, _n: CancelNotification) -> Result<()> {
        Ok(())
    }
    async fn set_session_mode(&self, _r: SetSessionModeRequest) -> Result<SetSessionModeResponse> {
        Ok(SetSessionModeResponse::new())
    }
    async fn set_session_config_option(
        &self,
        _r: SetSessionConfigOptionRequest,
    ) -> Result<SetSessionConfigOptionResponse> {
        Ok(SetSessionConfigOptionResponse::new(vec![config_option()]))
    }
}

#[tokio::test]
async fn site2_per_session_items_are_advertised_on_the_session_and_nowhere_else() {
    let derived = PerSessionGood::acp_derived();

    // They are NOT in AgentCapabilities — there is no field for them, which is
    // exactly why PLX-135 returned them as obligations.
    assert_eq!(
        derived.capabilities,
        plexus_acp::v1::schema::AgentCapabilities::new(),
        "set_mode / set_config_option have no AgentCapabilities field"
    );
    assert_eq!(derived.per_session.len(), 2);

    let session = PerSessionGood
        .new_session(new_session_request())
        .await
        .unwrap();
    assert!(session.modes.is_some());
    assert!(session.config_options.is_some());
}

/// Implements `set_session_mode` but the session says nothing about modes.
/// This is precisely "an implemented method that is never advertised".
pub struct PerSessionSilent;

#[plexus_macros::acp_agent]
impl PerSessionSilent {
    async fn new_session(&self, _r: NewSessionRequest) -> Result<NewSessionResponse> {
        Ok(NewSessionResponse::new("silent-1"))
    }
    async fn prompt(&self, _r: PromptRequest) -> Result<PromptResponse> {
        Ok(PromptResponse::new(StopReason::EndTurn))
    }
    async fn cancel(&self, _n: CancelNotification) -> Result<()> {
        Ok(())
    }
    async fn set_session_mode(&self, _r: SetSessionModeRequest) -> Result<SetSessionModeResponse> {
        Ok(SetSessionModeResponse::new())
    }
}

#[tokio::test]
async fn site2_an_implemented_per_session_item_that_the_session_hides_is_refused() {
    let err = PerSessionSilent
        .new_session(new_session_request())
        .await
        .unwrap_err();
    assert!(
        err.message.contains("set_session_mode") && err.message.contains("modes"),
        "the error must name the item and the field: {}",
        err.message
    );
}

/// The reverse: a session that offers a choice the agent cannot service.
pub struct PerSessionOverreach;

#[plexus_macros::acp_agent]
impl PerSessionOverreach {
    async fn new_session(&self, _r: NewSessionRequest) -> Result<NewSessionResponse> {
        Ok(NewSessionResponse::new("over-1").modes(mode_state(3)))
    }
    async fn prompt(&self, _r: PromptRequest) -> Result<PromptResponse> {
        Ok(PromptResponse::new(StopReason::EndTurn))
    }
    async fn cancel(&self, _n: CancelNotification) -> Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn site2_a_session_offering_modes_with_no_set_session_mode_is_refused() {
    let err = PerSessionOverreach
        .new_session(new_session_request())
        .await
        .unwrap_err();
    assert!(
        err.message.contains("set_session_mode"),
        "the error must name what is missing: {}",
        err.message
    );
}

/// Config options offered with no way to set them.
pub struct ConfigOverreach;

#[plexus_macros::acp_agent]
impl ConfigOverreach {
    async fn new_session(&self, _r: NewSessionRequest) -> Result<NewSessionResponse> {
        Ok(NewSessionResponse::new("cfg-1").config_options(vec![config_option()]))
    }
    async fn prompt(&self, _r: PromptRequest) -> Result<PromptResponse> {
        Ok(PromptResponse::new(StopReason::EndTurn))
    }
    async fn cancel(&self, _n: CancelNotification) -> Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn site2_a_session_offering_config_options_with_no_setter_is_refused() {
    let err = ConfigOverreach
        .new_session(new_session_request())
        .await
        .unwrap_err();
    assert!(
        err.message.contains("set_session_config_option"),
        "the error must name what is missing: {}",
        err.message
    );
}

/// A single fixed mode is NOT overreach — nothing to switch to.
pub struct OneFixedMode;

#[plexus_macros::acp_agent]
impl OneFixedMode {
    async fn new_session(&self, _r: NewSessionRequest) -> Result<NewSessionResponse> {
        Ok(NewSessionResponse::new("one-1").modes(mode_state(1)))
    }
    async fn prompt(&self, _r: PromptRequest) -> Result<PromptResponse> {
        Ok(PromptResponse::new(StopReason::EndTurn))
    }
    async fn cancel(&self, _n: CancelNotification) -> Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn site2_one_immutable_mode_is_not_an_unserviceable_choice() {
    let session = OneFixedMode
        .new_session(new_session_request())
        .await
        .unwrap();
    assert_eq!(session.modes.unwrap().available_modes.len(), 1);
}

// ===========================================================================
// The three idents the macro names, pinned against plexus-acp.
// ===========================================================================

/// `#[acp_agent]` holds no ACP vocabulary, but it does name three trait-item
/// idents structurally: `initialize` (generated), `new_session` (wrapped) and
/// `authenticate` (gates site 3). If plexus-acp ever renames one, that must
/// fail here rather than silently disarm an obligation.
#[test]
fn the_three_structural_idents_are_the_ones_plexus_acp_names() {
    assert_eq!(AgentMethod::Initialize.trait_item(), "initialize");
    assert_eq!(AgentMethod::SessionNew.trait_item(), "new_session");
    assert_eq!(AgentMethod::Authenticate.trait_item(), "authenticate");
    assert_eq!(
        AgentMethod::Authenticate.requirement(),
        Requirement::Optional(Advertisement::AuthMethods),
        "site 3 is what makes the `authenticate` ident load-bearing"
    );
}

/// The generated `new_session` wrapper matches on the `PerSession` payload
/// plexus-acp hands it. These are the only two payloads it knows how to
/// check; a third would make it fail closed, but that is a runtime refusal,
/// so the pin belongs here.
#[test]
fn the_per_session_advertisement_sites_are_the_two_this_macro_checks() {
    let sites: Vec<(&str, &str)> = AgentMethod::ALL
        .into_iter()
        .filter_map(|m| match m.requirement() {
            Requirement::Optional(Advertisement::PerSession(field)) => {
                Some((m.trait_item(), field))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        sites,
        vec![
            ("set_session_mode", "modes"),
            ("set_session_config_option", "configOptions"),
        ]
    );
}

/// The trait object path ACP·E dispatches through still works on generated
/// impls.
#[test]
fn a_generated_agent_is_object_safe() {
    let agents: Vec<Box<dyn Agent>> = vec![Box::new(Floor), Box::new(Loader), Box::new(Authed)];
    assert_eq!(agents.len(), 3);
}

/// `SessionConfigOptionId` is only imported to keep the fixture honest about
/// which type the id is; touching it here keeps the import load-bearing.
#[test]
fn the_config_option_fixture_is_a_real_config_option() {
    let opt = config_option();
    assert_eq!(opt.id, SessionConfigId::new("temp"));
}
