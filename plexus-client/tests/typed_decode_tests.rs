//! PERF.115-36: a typed client decodes an item's content straight from the
//! wire's text ([`decode_raw`]), never into a `Value` first. It must decode
//! what [`decode`] decodes through a `Value`, on every shape an activation's
//! event takes, and say what `decode` says when it cannot.

use futures::StreamExt;
use plexus_client::{
    decode, decode_raw, ClientError, ClientEvent, Content, ItemStream, MemoryTransport, MethodRef, RawItem,
    RawItemStream, StreamItem, Transport,
};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::value::RawValue;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::fmt::Debug;
use std::sync::Arc;

type Events<E> = Vec<Result<ClientEvent<E>, ClientError>>;

/// What `decode` makes of one data item whose content is `json`, through a
/// `Value`, and what `decode_raw` makes of it from the text.
fn both<E: DeserializeOwned + Debug + PartialEq + 'static>(json: &str) -> (Events<E>, Events<E>) {
    let value: Value = serde_json::from_str(json).expect("the content is JSON");
    let through_value: ItemStream<'static> = Box::pin(futures::stream::iter(vec![
        Ok(StreamItem::Data { metadata: None, content_type: Some("demo.watch".into()), content: value }),
        Ok(StreamItem::done()),
    ]));
    let text = RawValue::from_string(json.to_owned()).expect("the content is JSON");
    let from_text: RawItemStream<'static> = Box::pin(futures::stream::iter(vec![
        Ok(RawItem::Data { content_type: Some(Arc::from("demo.watch")), content: Content::from(text) }),
        Ok(RawItem::Done),
    ]));
    let a = futures::executor::block_on(decode::<E>(through_value).collect::<Vec<_>>());
    let b = futures::executor::block_on(decode_raw::<E>(from_text).collect::<Vec<_>>());
    (a, b)
}

/// The two decodes agree on `json`; returns what they made of it.
fn same<E: DeserializeOwned + Debug + PartialEq + 'static>(json: &str) -> Events<E> {
    let (a, b) = both::<E>(json);
    assert_eq!(a, b, "{json}");
    a
}

/// The two agree, and it decoded.
fn decodes<E: DeserializeOwned + Debug + PartialEq + 'static>(json: &str) {
    let got = same::<E>(json);
    assert!(matches!(got[..], [Ok(ClientEvent::Data(_))]), "{json}: {got:?}");
}

/// The two agree, and it did not.
fn refuses<E: DeserializeOwned + Debug + PartialEq + 'static>(json: &str) {
    let got = same::<E>(json);
    assert!(matches!(got[..], [Err(ClientError::Decode { .. })]), "{json}: {got:?}");
}

/// The house's event: tagged by `type`, snake case (every menger activation).
#[derive(Debug, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum House {
    Health { version: String, count: u64 },
    Item { id: String, parent: Option<String> },
    Ready,
    Nested { ticket: Ticket },
    Error { message: String },
}

#[derive(Debug, PartialEq, Deserialize)]
struct Ticket {
    id: String,
    title: String,
    #[serde(default)]
    labels: Vec<String>,
    #[serde(rename = "childCount")]
    child_count: u32,
    severity: Option<String>,
}

#[derive(Debug, PartialEq, Deserialize)]
enum External {
    Unit,
    Newtype(u8),
    Tuple(i32, String),
    Struct { a: bool },
}

#[derive(Debug, PartialEq, Deserialize)]
#[serde(tag = "t", content = "c")]
enum Adjacent {
    One(u8),
    Two { s: String },
}

#[derive(Debug, PartialEq, Deserialize)]
#[serde(untagged)]
enum Untagged {
    N(u64),
    S(String),
    Obj { a: bool },
}

#[derive(Debug, PartialEq, Deserialize)]
struct Flattened {
    id: String,
    #[serde(flatten)]
    rest: BTreeMap<String, Value>,
}

#[derive(Debug, PartialEq, Deserialize)]
struct Newtype(String);

#[derive(Debug, PartialEq, Deserialize)]
struct Pair(u8, String);

#[derive(Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct Strict {
    a: u8,
}

fn seven() -> u32 {
    7
}

#[derive(Debug, PartialEq, Deserialize)]
struct Defaults {
    #[serde(default)]
    n: u32,
    #[serde(default = "seven")]
    m: u32,
    #[serde(skip)]
    skipped: u8,
    #[serde(alias = "old")]
    new: String,
}

#[test]
fn the_houses_shape_decodes_the_same() {
    decodes::<House>(r#"{"type":"health","version":"0.7.0","count":3}"#);
    decodes::<House>(r#"{"type":"item","id":"A-1","parent":null}"#);
    decodes::<House>(r#"{"type":"item","id":"A-2","parent":"A-1","unknown":{"deep":[1,2]}}"#);
    decodes::<House>(r#"{"type":"ready"}"#);
    decodes::<House>(r#"{"count":3,"version":"x","type":"health"}"#);
    decodes::<House>(
        r#"{"type":"nested","ticket":{"id":"MQ-0","title":"a \"quoted\" title\n","labels":["perf","é"],"childCount":2,"severity":null}}"#,
    );
    decodes::<House>(r#"{"type":"error","message":"after: no ticket ZZZ-9"}"#);
}

#[test]
fn every_other_representation_decodes_the_same() {
    decodes::<External>(r#""Unit""#);
    decodes::<External>(r#"{"Newtype":1}"#);
    decodes::<External>(r#"{"Tuple":[-3,"x"]}"#);
    decodes::<External>(r#"{"Struct":{"a":true}}"#);
    decodes::<Adjacent>(r#"{"t":"One","c":4}"#);
    decodes::<Adjacent>(r#"{"c":{"s":"y"},"t":"Two"}"#);
    decodes::<Untagged>("18446744073709551615");
    decodes::<Untagged>(r#""s""#);
    decodes::<Untagged>(r#"{"a":false}"#);
    decodes::<Flattened>(r#"{"id":"x","b":1,"a":[2,{"c":null}]}"#);
    decodes::<Newtype>(r#""n""#);
    decodes::<Pair>(r#"[1,"p"]"#);
    decodes::<Defaults>(r#"{"old":"was"}"#);
    decodes::<Defaults>(r#"{"n":1,"m":2,"new":"is"}"#);
}

#[test]
fn scalars_and_collections_decode_the_same() {
    decodes::<String>(r#""tab\tquote\"é 😀 \\ \/""#);
    decodes::<u64>("9007199254740993");
    decodes::<i64>("-42");
    decodes::<f64>("0.1");
    decodes::<f64>("1e-7");
    decodes::<f32>("3.4028235e38");
    decodes::<bool>("true");
    decodes::<()>("null");
    decodes::<char>(r#""é""#);
    decodes::<Vec<u8>>("[1,2,3]");
    decodes::<Option<u8>>("null");
    decodes::<Option<u8>>("4");
    decodes::<BTreeMap<String, u8>>(r#"{"b":2,"a":1}"#);
    decodes::<HashMap<String, Vec<u8>>>(r#"{"x":[1],"y":[]}"#);
    decodes::<Value>(r#"{"z":[1,{"y":null}],"a":"x","n":-1.5}"#);
    decodes::<Box<Ticket>>(r#"{"id":"MQ-1","title":"t","childCount":0,"severity":"High"}"#);
}

#[test]
fn what_cannot_be_decoded_is_refused_in_the_same_words() {
    refuses::<House>(r#"{"type":"health","version":"0.7.0"}"#);
    refuses::<House>(r#"{"type":"health","version":7,"count":3}"#);
    refuses::<House>(r#"{"type":"gone"}"#);
    refuses::<House>(r#"{"version":"x"}"#);
    refuses::<u8>("300");
    refuses::<u8>("-1");
    refuses::<Strict>(r#"{"a":1,"b":2}"#);
    refuses::<External>(r#"{"Newtype":1.5}"#);
    refuses::<External>(r#""Nope""#);
    refuses::<Untagged>("[1]");
    refuses::<Pair>("[1]");
    refuses::<String>("12");
    refuses::<Ticket>(r#"{"id":"MQ-1"}"#);
}

#[test]
fn a_decode_error_names_the_content_type_and_no_position() {
    let got = same::<House>(r#"{"type":"health","version":"0.7.0"}"#);
    assert_eq!(
        got,
        vec![Err(ClientError::Decode { content_type: Some("demo.watch".into()), message: "missing field `count`".into() })]
    );
}

/// A transport with no text (one in process, a script) is read by
/// `decode_raw` from the `Value` it holds, as `decode` reads it.
#[tokio::test]
async fn a_transport_with_no_text_is_read_as_before() {
    const HEALTH: MethodRef = MethodRef::new("demo", "health");
    let t = MemoryTransport::new().script(
        HEALTH,
        vec![
            StreamItem::data(json!({"type": "health", "version": "0.7.0", "count": 1})),
            StreamItem::Progress { metadata: None, message: "half".into(), percentage: Some(50.0) },
            StreamItem::Request { request_id: "r".into(), request_data: json!({}), timeout_ms: 0 },
            StreamItem::data(json!({"type": "ready"})),
            StreamItem::Error { metadata: None, message: "no".into(), code: Some("forbidden".into()), recoverable: false },
        ],
    );
    let through_value: Events<House> = decode::<House>(t.call(HEALTH, json!({}))).collect().await;
    let raw: Events<House> = decode_raw::<House>(t.call_raw(HEALTH, json!({}))).collect().await;
    assert_eq!(through_value, raw);
    assert_eq!(raw.len(), 4, "{raw:?}");
}

/// A transport that has the text says so, through every handle a client
/// may be given it by: `&T`, `Box<T>`, `Rc<T>`, `Arc<T>`.
#[derive(Clone)]
struct Texts;

impl Transport for Texts {
    fn call(&self, _: MethodRef, _: Value) -> ItemStream<'_> {
        panic!("a typed client reads call_raw")
    }

    fn call_raw(&self, _: MethodRef, _: Value) -> RawItemStream<'_> {
        let text = RawValue::from_string(r#"{"type":"ready"}"#.into()).unwrap();
        Box::pin(futures::stream::iter(vec![Ok(RawItem::Data { content_type: None, content: Content::from(text) })]))
    }
}

fn read<T: Transport>(t: &T) -> Events<House> {
    futures::executor::block_on(decode_raw::<House>(t.call_raw(MethodRef::new("demo", "x"), json!({}))).collect())
}

#[test]
fn call_raw_reaches_the_transport_through_every_handle() {
    let want: Events<House> = vec![Ok(ClientEvent::Data(House::Ready))];
    assert_eq!(read(&&Texts), want);
    assert_eq!(read(&Box::new(Texts)), want);
    assert_eq!(read(&std::rc::Rc::new(Texts)), want);
    assert_eq!(read(&Arc::new(Texts)), want);
    let dynamic: Box<dyn Transport + Send + Sync> = Box::new(Texts);
    assert_eq!(read(&dynamic), want);
}
