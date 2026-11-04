//! MACRO-13 · `#[activation]` needs nothing.
//!
//! A bare attribute on `impl EchoActivation`: the stem `Echo` names the
//! namespace (`echo`), the trait (`trait Echo`, always emitted) and
//! the client (`EchoClient<T>`); every `pub async fn` on `&self` returning a
//! stream is a wire method, with nothing marked; descriptions come from `///`
//! alone. The schema is the one the same methods published before MACRO-13
//! (declared on `impl Echo` with `#[method]` and `namespace = "echo"`), and
//! the client's methods are the ones MACRO-4 emitted for that type.

use futures::{Stream, StreamExt};
use futures::TryStreamExt;
use plexus_client::{ClientError, FromClientError, MemoryTransport, MethodRef, StreamItem, Transport};
use plexus_core::plexus::{Activation, InProcessTransport};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EchoEvent {
    Said { text: String },
    Counted { n: u32 },
    Error { message: String },
}

impl FromClientError for EchoEvent {
    fn from_client_error(error: ClientError) -> Self {
        EchoEvent::Error { message: error.to_string() }
    }
}

/// The MACRO-13 form: nothing in the attribute.
mod now {
    use super::EchoEvent;
    use async_stream::stream;
    use futures::Stream;

    #[derive(Clone)]
    pub struct EchoActivation {
        step: u32,
    }

    /// Echo things back.
    #[plexus_macros::activation]
    impl EchoActivation {
        /// A plain constructor: public, not async, not on the wire.
        pub fn new() -> Self {
            Self { step: 1 }
        }

        /// Say a thing back.
        pub async fn say(
            &self,
            /// what to say
            text: String,
        ) -> impl Stream<Item = EchoEvent> + Send + 'static {
            stream! { yield EchoEvent::Said { text }; }
        }

        /// Count from one to `n`.
        pub async fn count(
            &self,
            /** how far to count */ n: u32,
        ) -> impl Stream<Item = EchoEvent> + Send + 'static {
            let step = self.step().await;
            stream! {
                let mut i = step;
                while i <= n {
                    yield EchoEvent::Counted { n: i };
                    i += step;
                }
            }
        }

        /// A private helper: not on the wire.
        async fn step(&self) -> u32 {
            self.step
        }
    }
}

/// The same methods as they were declared before MACRO-13.
mod before {
    use super::EchoEvent;
    use async_stream::stream;
    use futures::Stream;

    #[derive(Clone)]
    pub struct Echo {
        step: u32,
    }

    impl Echo {
        pub fn new() -> Self {
            Self { step: 1 }
        }
    }

    /// Echo things back.
    #[plexus_macros::activation(namespace = "echo", client)]
    impl Echo {
        /// Say a thing back.
        #[plexus_macros::method]
        async fn say(
            &self,
            /// what to say
            text: String,
        ) -> impl Stream<Item = EchoEvent> + Send + 'static {
            stream! { yield EchoEvent::Said { text }; }
        }

        /// Count from one to `n`.
        #[plexus_macros::method]
        async fn count(
            &self,
            /** how far to count */ n: u32,
        ) -> impl Stream<Item = EchoEvent> + Send + 'static {
            let step = self.step;
            stream! {
                let mut i = step;
                while i <= n {
                    yield EchoEvent::Counted { n: i };
                    i += step;
                }
            }
        }
    }
}

use now::{Echo, EchoActivation, EchoClient};

#[test]
fn a_bare_activation_publishes_namespace_echo_with_exactly_its_two_public_stream_methods() {
    let a = EchoActivation::new();
    assert_eq!(EchoActivation::NAMESPACE, "echo");
    assert_eq!(a.namespace(), "echo");
    assert_eq!(a.methods(), vec!["say", "count", "schema"]);
    let schema = a.plugin_schema();
    // `schema` is the built-in every activation answers; the two are its own.
    let names: Vec<&str> = schema.methods.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(names, vec!["say", "count", "schema"]);
    assert_eq!(schema.description, "Echo things back.");
    assert_eq!(EchoClient::<MemoryTransport>::NAMESPACE, "echo");
}

#[test]
fn the_schema_is_byte_for_byte_the_one_the_same_methods_published_before() {
    let now = serde_json::to_string(&EchoActivation::new().plugin_schema()).unwrap();
    let before = serde_json::to_string(&before::Echo::new().plugin_schema()).unwrap();
    assert_eq!(now, before);
    // and the descriptions are there, from `///` alone
    assert!(now.contains("Say a thing back."), "{now}");
    assert!(now.contains("what to say"), "{now}");
    assert!(now.contains("how far to count"), "{now}");
}

#[test]
fn the_client_is_the_one_macro_4_emitted_for_a_type_named_echo() {
    // The same consts …
    assert_eq!(EchoClient::<MemoryTransport>::METHOD_SAY, before::EchoClient::<MemoryTransport>::METHOD_SAY);
    assert_eq!(EchoClient::<MemoryTransport>::METHOD_COUNT, before::EchoClient::<MemoryTransport>::METHOD_COUNT);

    // … the same signatures (a change to either is a compile error here) …
    fn now_sigs<T: Transport>(c: &EchoClient<T>) {
        let _: plexus_client::MethodStream<'_, EchoEvent, _, _> = c.say(String::new());
        let _: plexus_client::MethodStream<'_, EchoEvent, _, _> = c.count(0);
        let _ = c.say(String::new()).try_collect::<Vec<_>>();
        let _ = c.count(0).try_collect::<Vec<_>>();
        let _: &T = c.transport();
    }
    fn before_sigs<T: Transport>(c: &before::EchoClient<T>) {
        let _: plexus_client::MethodStream<'_, EchoEvent, _, _> = c.say(String::new());
        let _: plexus_client::MethodStream<'_, EchoEvent, _, _> = c.count(0);
        let _ = c.say(String::new()).try_collect::<Vec<_>>();
        let _ = c.count(0).try_collect::<Vec<_>>();
        let _: &T = c.transport();
    }
    let _ = (now_sigs::<MemoryTransport>, before_sigs::<MemoryTransport>);

    // … and the same call on the wire.
    let say = MethodRef::new("echo", "say");
    let t1 = MemoryTransport::new().script_events(say, &[EchoEvent::Said { text: "x".into() }]);
    let t2 = MemoryTransport::new().script_events(say, &[EchoEvent::Said { text: "x".into() }]);
    let (a, b) = futures::executor::block_on(async {
        (
            EchoClient::new(&t1).say("x".into()).try_collect::<Vec<_>>().await.unwrap(),
            before::EchoClient::new(&t2).say("x".into()).try_collect::<Vec<_>>().await.unwrap(),
        )
    });
    assert_eq!(a, b);
    assert_eq!(t1.params_of(say), t2.params_of(say));
}

fn is_echo<T: Echo>() {}

#[tokio::test]
async fn the_activation_and_its_client_are_both_an_echo_and_round_trip_through_it() {
    is_echo::<EchoActivation>();
    is_echo::<EchoClient<InProcessTransport<EchoActivation>>>();

    let echoes: Vec<Box<dyn Echo>> = vec![
        Box::new(EchoActivation::new()),
        Box::new(EchoClient::new(InProcessTransport::new(EchoActivation::new()))),
    ];
    for echo in &echoes {
        let said: Vec<EchoEvent> = echo.say("hi".into()).collect().await;
        assert_eq!(said, vec![EchoEvent::Said { text: "hi".into() }]);
        let counted: Vec<EchoEvent> = echo.count(3).collect().await;
        assert_eq!(
            counted,
            vec![EchoEvent::Counted { n: 1 }, EchoEvent::Counted { n: 2 }, EchoEvent::Counted { n: 3 }]
        );
    }
}

#[tokio::test]
async fn a_client_failure_is_the_event_types_own_error_and_ends_the_stream() {
    let say = MethodRef::new("echo", "say");
    let t = MemoryTransport::new().script(
        say,
        vec![
            StreamItem::Progress { metadata: None, content_type: None, content: serde_json::json!({"message": "thinking"}) },
            StreamItem::Error {
                metadata: None,
                recoverable: false,
                message: Some("no".into()),
                code: None,
                error: plexus_client::RemoteError::Execution { message: "no".into() },
                fields: Default::default(),
            },
            StreamItem::data(serde_json::json!({"type": "said", "text": "late"})),
        ],
    );
    let client: Box<dyn Echo> = Box::new(EchoClient::new(Arc::new(t)));
    let got: Vec<EchoEvent> = client.say("hi".into()).collect().await;
    assert_eq!(got, vec![EchoEvent::Error { message: "remote error: no".into() }]);
}

/// `namespace = "other"` overrides the stem; `methods = marked` publishes
/// only what is marked, and lets a public async method return anything.
mod overrides {
    use super::EchoEvent;
    use async_stream::stream;
    use futures::Stream;

    #[derive(Clone)]
    pub struct EchoActivation;

    /// Another echo.
    #[plexus_macros::activation(namespace = "other")]
    impl EchoActivation {
        /// Say.
        pub async fn say(&self, text: String) -> impl Stream<Item = EchoEvent> + Send + 'static {
            stream! { yield EchoEvent::Said { text }; }
        }
    }

    #[derive(Clone)]
    pub struct MarkedActivation;

    /// Only one on the wire.
    #[plexus_macros::activation(methods = marked)]
    impl MarkedActivation {
        /// On the wire.
        #[plexus_macros::method]
        pub async fn say(&self, text: String) -> impl Stream<Item = EchoEvent> + Send + 'static {
            stream! { yield EchoEvent::Said { text }; }
        }

        /// Public, a stream, and not marked: off the wire.
        pub async fn quiet(&self) -> impl Stream<Item = EchoEvent> + Send + 'static {
            stream! { yield EchoEvent::Counted { n: 0 }; }
        }

        /// Public and async, not a stream: fine, under `methods = marked`.
        pub async fn total(&self) -> u32 {
            7
        }
    }
}

#[test]
fn namespace_overrides_the_stem_and_methods_marked_restores_the_marked_rule() {
    use overrides::{EchoActivation as Other, MarkedActivation};
    assert_eq!(Other.namespace(), "other");
    assert_eq!(overrides::EchoClient::<MemoryTransport>::NAMESPACE, "other");

    assert_eq!(MarkedActivation.namespace(), "marked");
    assert_eq!(MarkedActivation.methods(), vec!["say", "schema"]);
    let _ = MarkedActivation.quiet();
    let _ = MarkedActivation.total();
}

/// What a method's stream type need not be: `impl Stream` here is enough.
#[allow(dead_code)]
fn streams_are_streams(s: impl Stream<Item = EchoEvent>) -> impl Stream<Item = EchoEvent> {
    s.map(|e| e)
}

/// A method that takes what the server injects (`auth`) is on the trait with
/// what a caller passes; the client implements the trait, and the activation
/// — which has no caller to take `auth` from — does not.
mod injected {
    use super::EchoEvent;
    use async_stream::stream;
    use futures::Stream;
    use plexus_core::plexus::AuthContext;

    #[derive(Clone)]
    pub struct GuardedActivation;

    /// Who is asking.
    #[plexus_macros::activation]
    impl GuardedActivation {
        /// Greet the caller.
        pub async fn greet(&self, auth: &AuthContext, greeting: String) -> impl Stream<Item = EchoEvent> + Send + 'static {
            let text = format!("{greeting}, {}", auth.user_id);
            stream! { yield EchoEvent::Said { text }; }
        }
    }
}

#[tokio::test]
async fn an_injected_parameter_stays_off_the_trait_and_the_client_implements_it() {
    use injected::{Guarded, GuardedActivation, GuardedClient};
    fn is_guarded<T: Guarded>() {}
    is_guarded::<GuardedClient<Arc<MemoryTransport>>>();
    assert_eq!(GuardedActivation.namespace(), "guarded");

    let greet = MethodRef::new("guarded", "greet");
    let t = Arc::new(MemoryTransport::new().script_events(greet, &[EchoEvent::Said { text: "hi, ann".into() }]));
    let client: Box<dyn Guarded> = Box::new(GuardedClient::new(t.clone()));
    let got: Vec<EchoEvent> = client.greet("hi".into()).collect().await;
    assert_eq!(got, vec![EchoEvent::Said { text: "hi, ann".into() }]);
    assert_eq!(t.params_of(greet), Some(serde_json::json!({"greeting": "hi"})));
}

/// A client mounted at a path (MACRO-16's `XClient::at`), called through the
/// trait, goes where its inherent method goes: `host.echo.say`, not `echo.say`.
#[test]
fn a_mounted_client_through_the_trait_keeps_its_path() {
    let at = EchoClient::<MemoryTransport>::METHOD_SAY.at("host.echo");
    let t = Arc::new(MemoryTransport::new().script_events(at, &[EchoEvent::Said { text: "x".into() }]));
    let client = EchoClient::at(t, "host.echo");
    let inherent = futures::executor::block_on(client.say("x".into()).try_collect::<Vec<_>>()).unwrap();
    assert_eq!(inherent, vec![EchoEvent::Said { text: "x".into() }]);
    let through: Vec<EchoEvent> = futures::executor::block_on(Echo::say(&client, "x".into()).collect());
    assert_eq!(through, vec![EchoEvent::Said { text: "x".into() }]);
}
