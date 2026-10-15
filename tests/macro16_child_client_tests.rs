//! MACRO-16: the emitted client reaches `#[child]` namespaces.
//!
//! For a parent with `#[child] fn inputs(&self) -> Inputs`, `TicketsClient`
//! gets `fn inputs(&self) -> InputsClient<&T>`, whose calls go to
//! `tickets.inputs.<method>` — the path the hub routes. A grandchild is two
//! accessors and a three-segment path. A child client built alone keeps
//! today's behaviour (`inputs.<method>`).

use plexus_client::{MemoryTransport, MethodRef};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Ok { at: String },
}

#[derive(Clone, Default)]
pub struct Tickets;

#[plexus_macros::activation(namespace = "tickets", description = "the parent", client)]
impl Tickets {
    /// A parent method.
    #[plexus_macros::method]
    pub async fn get(&self, id: String) -> impl futures::Stream<Item = Event> + Send + 'static {
        futures::stream::iter(vec![Event::Ok { at: id }])
    }

    /// The inputs namespace.
    #[plexus_macros::child]
    pub fn inputs(&self) -> Inputs {
        Inputs
    }

    /// A dynamic child gets no accessor (its segment is a runtime name).
    #[plexus_macros::child]
    pub fn named(&self, name: &str) -> Option<Inputs> {
        (name == "x").then_some(Inputs)
    }
}

#[derive(Clone, Default)]
pub struct Inputs;

#[plexus_macros::activation(namespace = "inputs", description = "the child", client)]
impl Inputs {
    /// A child method.
    #[plexus_macros::method]
    pub async fn answer(&self, ticket: String, value: String) -> impl futures::Stream<Item = Event> + Send + 'static {
        futures::stream::iter(vec![Event::Ok { at: format!("{ticket}={value}") }])
    }

    /// A grandchild, whose namespace differs from its accessor's name: the
    /// route segment is the accessor's.
    #[plexus_macros::child]
    pub fn history(&self) -> self::deep::Log {
        deep::Log
    }
}

pub mod deep {
    use super::Event;

    #[derive(Clone, Default)]
    pub struct Log;

    #[plexus_macros::activation(namespace = "log", description = "the grandchild", client)]
    impl Log {
        /// A grandchild method.
        #[plexus_macros::method]
        pub async fn tail(&self, n: Option<u32>) -> impl futures::Stream<Item = Event> + Send + 'static {
            futures::stream::iter(vec![Event::Ok { at: format!("{n:?}") }])
        }
    }
}

fn names(t: &MemoryTransport) -> Vec<String> {
    t.calls().into_iter().map(|(m, _)| m).collect()
}

#[tokio::test]
async fn the_child_accessor_sends_the_routed_path() {
    let t = MemoryTransport::new();
    let client = TicketsClient::new(&t);
    let _ = client.inputs().answer_all("ACCESS-6".into(), "In Review".into()).await;
    let calls = t.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "tickets.inputs.answer");
    assert_eq!(calls[0].1, json!({ "ticket": "ACCESS-6", "value": "In Review" }));
    assert_eq!(client.inputs().path(), "tickets.inputs");
}

#[tokio::test]
async fn a_grandchild_sends_three_segments() {
    let t = MemoryTransport::new();
    let client = TicketsClient::new(&t);
    let _ = client.inputs().history().tail_all(Some(3)).await;
    assert_eq!(names(&t), vec!["tickets.inputs.history.tail"]);
    assert_eq!(client.inputs().history().path(), "tickets.inputs.history");
}

#[tokio::test]
async fn the_parent_and_a_child_alone_are_unchanged() {
    let t = MemoryTransport::new();
    let _ = TicketsClient::new(&t).get_all("MNG-1".into()).await;
    let _ = InputsClient::new(&t).answer_all("MNG-1".into(), "x".into()).await;
    assert_eq!(names(&t), vec!["tickets.get", "inputs.answer"]);
    assert_eq!(TicketsClient::new(&t).path(), "tickets");
}

#[tokio::test]
async fn a_client_built_at_a_path_sends_beneath_it() {
    let t = MemoryTransport::new();
    let _ = InputsClient::at(&t, "tickets.inputs").answer_all("A-1".into(), "v".into()).await;
    let _ = InputsClient::at(&t, "inputs").answer_all("A-1".into(), "v".into()).await;
    assert_eq!(names(&t), vec!["tickets.inputs.answer", "inputs.answer"]);
}

#[test]
fn method_ref_at_a_path() {
    let m = MethodRef::new("inputs", "answer");
    assert_eq!(m.qualified(), "inputs.answer");
    assert_eq!(m.path(), "inputs");
    let at = m.at("tickets.inputs");
    assert_eq!(at.qualified(), "tickets.inputs.answer");
    assert_eq!(at.to_string(), "tickets.inputs.answer");
    assert_eq!(at.namespace(), "inputs");
    assert_eq!(at.method(), "answer");
    // a path equal to the namespace, or empty, is the reference itself
    assert_eq!(m.at("inputs"), m);
    assert_eq!(m.at(""), m);
}

#[test]
fn joined_paths_are_interned() {
    let a = plexus_client::__rt::join_path("tickets", "inputs");
    let b = plexus_client::__rt::join_path("tickets", "inputs");
    assert_eq!(a, "tickets.inputs");
    assert!(std::ptr::eq(a, b), "one allocation per distinct pair");
    assert_eq!(plexus_client::__rt::join_path("", "tickets"), "tickets");
}
