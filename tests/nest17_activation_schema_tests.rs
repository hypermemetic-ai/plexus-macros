//! NEST-17 · the activation's trait every time, `{Name}ActivationSchema`
//! by default; paths, not strings, in `#[child]` and `#[peer]`.
//!
//! - `impl Shop` with no attribute arguments: namespace `shop`,
//!   `ShopClient`, `ShopActivationSchema`, implemented by `Shop` and — every
//!   method answering one `Result<T, E>`, `E: From<PeerError>` — by
//!   `ShopClient<T>`, so `dyn ShopActivationSchema` is local or remote alike.
//! - A method that takes what the server injects (`auth`, `reach`,
//!   `#[from_auth]`) keeps the trait: the trait form reads them from the
//!   call's context; outside a call a required caller is the method's error,
//!   `reach` is nowhere.
//! - `trait = "…"` renames it; an activation that is not `Clone` gets the
//!   trait and no impl; `list = names` names a sibling by path, and the
//!   string form still builds.

use async_stream::stream;
use futures::{Stream, StreamExt};
use plexus_client::{MemoryTransport, MethodRef, StreamItem, TransportError};
use plexus_core::plexus::peer::PeerError;
use plexus_core::plexus::{AuthContext, Reach};
use plexus_core::DynamicHub;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;

// ------------------------------------------------------------------ the shop

/// Why the shop said no — or could not be reached.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ShopError {
    Unknown { item: String },
    Peer { error: PeerError },
}

impl From<PeerError> for ShopError {
    fn from(error: PeerError) -> Self {
        ShopError::Peer { error }
    }
}

#[derive(Clone)]
pub struct Shop;

/// A shop: every method one `Result`, so its client is the trait too.
#[plexus_macros::activation]
impl Shop {
    /// An item's price.
    #[plexus_macros::method]
    pub async fn price(&self, item: String) -> Result<u64, ShopError> {
        match item.as_str() {
            "tea" => Ok(4),
            _ => Err(ShopError::Unknown { item }),
        }
    }

    /// Who is asking — the caller, read from the call in the trait form.
    #[plexus_macros::method]
    pub async fn whoami(&self, auth: &AuthContext) -> Result<String, ShopError> {
        Ok(auth.user_id.clone())
    }
}

/// The trait by the stem, `ShopActivation` → `ShopActivationSchema` too.
#[derive(Clone)]
pub struct TillActivation;

#[plexus_macros::activation]
impl TillActivation {
    /// What the till saw of its call.
    pub async fn reach(&self, reach: Reach) -> impl Stream<Item = bool> + Send + 'static {
        let everywhere = reach.is_everywhere();
        stream! { yield everywhere; }
    }

    /// Whether anyone is signed in.
    pub async fn signed_in(&self, auth: Option<&AuthContext>) -> impl Stream<Item = bool> + Send + 'static {
        let signed = auth.is_some();
        stream! { yield signed; }
    }
}

/// Not `Clone`: the trait, and no impl of it — the crate still builds.
pub struct Ledger;

#[plexus_macros::activation]
impl Ledger {
    /// Lines.
    pub async fn lines(&self) -> impl Stream<Item = u32> + Send + 'static {
        stream! { yield 1; }
    }
}

/// `trait = "…"` renames the trait (and the client and namespace).
#[derive(Clone)]
pub struct Counter;

#[plexus_macros::activation(trait = "Tally")]
impl Counter {
    /// Up to `n`.
    pub async fn up(&self, n: u32) -> impl Stream<Item = u32> + Send + 'static {
        stream! { for i in 1..=n { yield i; } }
    }
}

/// A host whose method calls the till through its trait, inside a call.
#[derive(Clone)]
pub struct Front {
    till: Arc<dyn TillActivationSchema>,
}

#[plexus_macros::activation]
impl Front {
    /// The till's answers, asked inside this call.
    pub async fn ask(&self) -> impl Stream<Item = bool> + Send + 'static {
        let reach: Vec<bool> = self.till.reach().collect().await;
        let signed: Vec<bool> = self.till.signed_in().collect().await;
        stream! { for b in reach.into_iter().chain(signed) { yield b; } }
    }

    /// Items by name: a dynamic child, its names by path.
    #[plexus_macros::child(list = names)]
    fn item(&self, name: &str) -> Option<Counter> {
        (name == "one").then_some(Counter)
    }

    /// The names.
    fn names(&self) -> impl Stream<Item = String> + Send + '_ {
        stream! { yield "one".to_string(); }
    }
}

/// The string form still builds (with a deprecation warning).
#[derive(Clone)]
pub struct Back;

#[plexus_macros::activation]
impl Back {
    /// Nothing.
    pub async fn nothing(&self) -> impl Stream<Item = u32> + Send + 'static {
        stream! { yield 0; }
    }

    /// Items by name.
    #[plexus_macros::child(list = "names")]
    fn item(&self, name: &str) -> Option<Counter> {
        (name == "one").then_some(Counter)
    }

    /// The names.
    fn names(&self) -> impl Stream<Item = String> + Send + '_ {
        stream! { yield "one".to_string(); }
    }
}

/// menger's shape: a request type and a `#[from_auth]` verifier on every
/// method — still the trait (`DeskActivationSchema`), and implemented.
#[derive(plexus_macros::PlexusRequest)]
pub struct DeskRequest {
    #[from_auth_context]
    pub auth: Option<AuthContext>,
}

#[derive(Clone)]
pub struct User(String);

#[derive(Clone)]
pub struct DeskActivation;

impl DeskActivation {
    async fn verify(&self, auth: &AuthContext) -> Result<User, plexus_core::plexus::PlexusError> {
        Ok(User(auth.user_id.clone()))
    }
}

#[plexus_macros::activation(request = DeskRequest)]
impl DeskActivation {
    /// The signed-in user's name, as a stream of one.
    pub async fn me(
        &self,
        #[from_auth(self.verify)] user: User,
    ) -> impl Stream<Item = Result<String, ShopError>> + Send + 'static {
        stream! { yield Ok(user.0); }
    }
}

fn implements<T: ?Sized>() {}

// ------------------------------------------------------------------ acceptance

#[test]
fn every_activation_has_its_trait() {
    implements::<dyn ShopActivationSchema>();
    implements::<dyn TillActivationSchema>();
    implements::<dyn LedgerActivationSchema>();
    implements::<dyn Tally>();
    implements::<dyn FrontActivationSchema>();
    fn is<T: ShopActivationSchema>() {}
    is::<Shop>();
    is::<ShopClient<Arc<MemoryTransport>>>();
    fn till<T: TillActivationSchema>() {}
    till::<TillActivation>();
    fn desk<T: DeskActivationSchema>() {}
    desk::<DeskActivation>();
    fn tally<T: Tally>() {}
    tally::<Counter>();
    assert_eq!(ShopClient::<()>::NAMESPACE, "shop");
    assert_eq!(TillClient::<()>::NAMESPACE, "till");
    assert_eq!(TallyClient::<()>::NAMESPACE, "tally");
}

#[tokio::test]
async fn a_local_shop_and_a_remote_one_behind_one_dyn() {
    let reply = |v: Value| vec![StreamItem::Data { metadata: None, content_type: Some("shop.price".into()), content: v }, StreamItem::done()];
    let t = Arc::new(
        MemoryTransport::new()
            .script(ShopClient::<Arc<MemoryTransport>>::METHOD_PRICE, reply(json!({ "ok": 4 })))
            .script_failure(MethodRef::new("shop", "whoami"), TransportError::protocol("the connection has closed")),
    );
    let shops: Vec<Box<dyn ShopActivationSchema>> = vec![Box::new(Shop), Box::new(ShopClient::new(t.clone()))];
    for s in &shops {
        assert_eq!(s.price("tea".into()).await, Ok(4));
    }
    // The remote one's failure in transit is the method's own error.
    match shops[1].whoami().await {
        Err(ShopError::Peer { error: PeerError::Unavailable { .. } }) => {}
        other => panic!("{other:?}"),
    }
    assert_eq!(t.params_of(ShopClient::<Arc<MemoryTransport>>::METHOD_PRICE), Some(json!({ "item": "tea" })));
}

#[tokio::test]
async fn outside_a_call_a_required_caller_is_the_methods_error() {
    let shop: &dyn ShopActivationSchema = &Shop;
    match shop.whoami().await {
        Err(ShopError::Peer { error: PeerError::Forbidden { grant } }) => assert_eq!(grant, "shop.whoami"),
        other => panic!("{other:?}"),
    }
    // A `#[from_auth]` method too: its stream says it, as its error.
    let desk: &dyn DeskActivationSchema = &DeskActivation;
    match desk.me().collect::<Vec<_>>().await.as_slice() {
        [Err(ShopError::Peer { error: PeerError::Forbidden { grant } })] => assert_eq!(grant, "desk.me"),
        other => panic!("{other:?}"),
    }
    // `reach` outside a call is nowhere; an optional caller is none.
    let till: &dyn TillActivationSchema = &TillActivation;
    assert_eq!(till.reach().collect::<Vec<_>>().await, vec![false]);
    assert_eq!(till.signed_in().collect::<Vec<_>>().await, vec![false]);
}

#[tokio::test]
async fn inside_a_call_the_trait_reads_the_calls_context() {
    let hub = DynamicHub::new("svc").register(Front { till: Arc::new(TillActivation) });
    let items: Vec<Value> = hub
        .route("front.ask", json!({}), None)
        .await
        .unwrap()
        .filter_map(|i| async move {
            match i {
                plexus_core::plexus::PlexusStreamItem::Data { content, .. } => Some(content.into_value()),
                _ => None,
            }
        })
        .collect()
        .await;
    // No gate: the dispatch's reach is everywhere; anonymous.
    assert_eq!(items, vec![json!(true), json!(false)]);
}

#[test]
fn list_by_path_and_by_string_both_name_the_sibling() {
    for schema in [Front { till: Arc::new(TillActivation) }.slot_schema_for_test(), Back.slot_schema_for_test()] {
        assert!(schema.contains("names"), "{schema}");
    }
}

trait SlotSchema {
    fn slot_schema_for_test(&self) -> String;
}

impl<A: plexus_core::plexus::Activation> SlotSchema for A {
    fn slot_schema_for_test(&self) -> String {
        serde_json::to_string(&self.plugin_schema()).unwrap()
    }
}
