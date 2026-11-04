//! PERF.115-36: a typed client decodes an item's content straight from the
//! wire's text ([`decode_raw`]), never into a `Value` first. It must decode
//! what [`decode`] decodes through a `Value`, on every shape an activation's
//! event takes, and say what `decode` says when it cannot.
//!
//! PERF.115-43: a house event derived with `plexus_macros::Deserialize` is
//! read with its `type` first, without serde's buffer. It must read what
//! serde's derive reads, on every shape the house's events take, and refuse
//! what serde refuses in serde's words ([`twins!`] declares each enum both
//! ways).

use futures::StreamExt;
use plexus_client::{
    decode, decode_raw, ClientError, Content, ItemStream, MemoryTransport, MethodRef, RawItem,
    RawItemStream, StreamItem, Transport,
};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::value::RawValue;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::fmt::Debug;
use std::sync::Arc;

type Events<E> = Vec<Result<E, ClientError>>;

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
    assert!(matches!(got[..], [Ok(_)]), "{json}: {got:?}");
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
            StreamItem::Progress { metadata: None, content_type: None, content: json!({"message": "half", "percentage": 50.0}) },
            StreamItem::Request { request_id: "r".into(), request_data: json!({}), timeout_ms: 0 },
            StreamItem::data(json!({"type": "ready"})),
            StreamItem::Error {
                metadata: None,
                recoverable: false,
                message: Some("no".into()),
                code: Some("-32003".into()),
                error: plexus_client::RemoteError::Forbidden { reason: plexus_client::DenyReason::NotAccepted },
                fields: Default::default(),
            },
        ],
    );
    let through_value: Events<House> = decode::<House>(t.call(HEALTH, json!({}))).collect().await;
    let raw: Events<House> = decode_raw::<House>(t.call_raw(HEALTH, json!({}))).collect().await;
    assert_eq!(through_value, raw);
    // RETURNS-36: two items and the error; progress dropped unopted, as a
    // MethodStream drops it.
    assert_eq!(raw.len(), 3, "{raw:?}");
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
    let want: Events<House> = vec![Ok(House::Ready)];
    assert_eq!(read(&&Texts), want);
    assert_eq!(read(&Box::new(Texts)), want);
    assert_eq!(read(&std::rc::Rc::new(Texts)), want);
    assert_eq!(read(&Arc::new(Texts)), want);
    let dynamic: Box<dyn Transport + Send + Sync> = Box::new(Texts);
    assert_eq!(read(&dynamic), want);
}

// ---------------------------------------------------------------------------
// PERF.115-43: a house event read with its `type` first.
// ---------------------------------------------------------------------------

/// An enum written once and declared twice: `serde_reads::Name`, read by
/// serde's derive, and `type_first::Name`, read by
/// `plexus_macros::Deserialize`. Named alike, so their values and serde's
/// words about them print alike.
macro_rules! twins {
    ($module:ident: $(#[$attr:meta])* enum $name:ident { $($body:tt)* }) => {
        mod $module {
            pub mod serde_reads {
                #[allow(unused_imports)]
                use crate::*;
                #[derive(Debug, PartialEq, serde::Deserialize)]
                $(#[$attr])*
                #[allow(dead_code)]
                pub(crate) enum $name { $($body)* }
            }
            pub mod type_first {
                #[allow(unused_imports)]
                use crate::*;
                #[derive(Debug, PartialEq, plexus_macros::Deserialize)]
                $(#[$attr])*
                #[allow(dead_code)]
                pub(crate) enum $name { $($body)* }
            }
        }
    };
}

/// A read as it prints: the value, or serde's words without the position a
/// read from text adds (`decode` drops it too).
fn printed<T: Debug>(read: Result<T, serde_json::Error>) -> Result<String, String> {
    read.map(|v| format!("{v:?}")).map_err(|e| {
        let message = e.to_string();
        let at = format!(" at line {} column {}", e.line(), e.column());
        match message.strip_suffix(&at) {
            Some(bare) if e.line() != 0 => bare.to_owned(),
            _ => message,
        }
    })
}

/// What a typed client's decode makes of one item whose content is `json`.
fn client_reads<E: DeserializeOwned + Debug + PartialEq + 'static>(json: &str) -> String {
    let text = RawValue::from_string(json.to_owned()).expect("the content is JSON");
    let items: RawItemStream<'static> = Box::pin(futures::stream::iter(vec![
        Ok(RawItem::Data { content_type: Some(Arc::from("demo.watch")), content: Content::from(text) }),
        Ok(RawItem::Done),
    ]));
    format!("{:?}", futures::executor::block_on(decode_raw::<E>(items).collect::<Vec<_>>()))
}

/// serde's read (`S`) and the read with the type first (`F`) make the same of
/// `json`: from text (keys borrowed), from a reader (keys owned), from a
/// `Value` (no position on either side) and through a typed client's decode.
/// Returns what serde makes of it from text.
fn alike<S, F>(json: &str) -> Result<String, String>
where
    S: DeserializeOwned + Debug + PartialEq + 'static,
    F: DeserializeOwned + Debug + PartialEq + 'static,
{
    let serde = printed(serde_json::from_str::<S>(json));
    assert_eq!(serde, printed(serde_json::from_str::<F>(json)), "from text: {json}");
    assert_eq!(
        printed(serde_json::from_reader::<_, S>(json.as_bytes())),
        printed(serde_json::from_reader::<_, F>(json.as_bytes())),
        "from a reader: {json}"
    );
    if let Ok(value) = serde_json::from_str::<Value>(json) {
        assert_eq!(
            printed(serde_json::from_value::<S>(value.clone())),
            printed(serde_json::from_value::<F>(value)),
            "from a Value: {json}"
        );
    }
    assert_eq!(client_reads::<S>(json), client_reads::<F>(json), "a typed client: {json}");
    serde
}

/// Both read `json`, alike.
fn both_read<S, F>(json: &str)
where
    S: DeserializeOwned + Debug + PartialEq + 'static,
    F: DeserializeOwned + Debug + PartialEq + 'static,
{
    let read = alike::<S, F>(json);
    assert!(read.is_ok(), "{json}: {read:?}");
}

/// Both refuse `json`, in the same words; returns them.
fn both_refuse<S, F>(json: &str) -> String
where
    S: DeserializeOwned + Debug + PartialEq + 'static,
    F: DeserializeOwned + Debug + PartialEq + 'static,
{
    match alike::<S, F>(json) {
        Err(words) => words,
        Ok(read) => panic!("{json}: read as {read}"),
    }
}

twins! { house:
    #[serde(tag = "type", rename_all = "snake_case")]
    enum House {
        Health { version: String, count: u64 },
        Item { id: String, parent: Option<String> },
        Ready,
        Nested { ticket: Ticket },
        Error { message: String },
    }
}

type HouseS = house::serde_reads::House;
type HouseF = house::type_first::House;

#[test]
fn a_house_event_reads_with_its_type_first_as_serde_reads_it() {
    for json in [
        r#"{"type":"health","version":"0.7.0","count":3}"#,
        r#"{"type":"item","id":"A-1","parent":null}"#,
        r#"{"type":"item","id":"A-1"}"#,
        r#"{"type":"item","id":"A-2","parent":"A-1","unknown":{"deep":[1,2,{"type":"x"}]},"more":"\"q\""}"#,
        r#"{"type":"ready"}"#,
        r#"{"type":"ready","extra":[1,{"x":null}],"n":-1.5e3}"#,
        r#"{"type":"nested","ticket":{"id":"MQ-0","title":"a \"quoted\" title\n","labels":["perf","é"],"childCount":2,"severity":null}}"#,
        r#"{"type":"error","message":"after: no ticket ZZZ-9"}"#,
        // the type not first: serde's read, as before
        r#"{"count":3,"version":"x","type":"health"}"#,
        r#"{"version":"x","type":"health","count":3}"#,
        r#"{"id":"A-3","type":"item"}"#,
        // spaces, and a key, a tag and a field written with escapes
        r#" { "type" : "health" , "version" : "x" , "count" : 3 } "#,
        r#"{"type":"health","version":"x","count":3}"#,
        r#"{"type":"health","version":"x","count":3}"#,
        r#"{"type":"health","version":"x","count":3}"#,
        // a sequence, its first element the tag: serde's read
        r#"["health","0.7.0",3]"#,
        r#"["ready"]"#,
    ] {
        both_read::<HouseS, HouseF>(json);
    }
}

#[test]
fn what_serde_refuses_the_type_first_refuses_in_its_words() {
    let cases = [
        (r#"{"type":"health","version":"0.7.0"}"#, "missing field `count`"),
        (r#"{"type":"health","version":7,"count":3}"#, "invalid type: integer `7`, expected a string"),
        (r#"{"type":"health","version":"x","version":"y","count":1}"#, "duplicate field `version`"),
        (
            r#"{"type":"gone"}"#,
            "unknown variant `gone`, expected one of `health`, `item`, `ready`, `nested`, `error`",
        ),
        (
            r#"{"type":"gone","version":"x"}"#,
            "unknown variant `gone`, expected one of `health`, `item`, `ready`, `nested`, `error`",
        ),
        (r#"{"type":5}"#, "invalid type: integer `5`, expected variant identifier"),
        (r#"{"type":null}"#, "invalid type: null, expected variant identifier"),
        (r#"{"type":{"a":1}}"#, "invalid type: map, expected variant identifier"),
        (r#"{"type":["health"]}"#, "invalid type: sequence, expected variant identifier"),
        (r#"{"version":"x"}"#, "missing field `type`"),
        (r#"{}"#, "missing field `type`"),
        // a second tag, wherever it comes
        (r#"{"type":"health","type":"health","version":"x","count":1}"#, "duplicate field `type`"),
        (r#"{"type":"health","version":"x","count":1,"type":"ready"}"#, "duplicate field `type`"),
        (r#"{"type":"ready","type":"ready"}"#, "duplicate field `type`"),
        (r#"{"type":"ready","a":1,"type":"x"}"#, "duplicate field `type`"),
        (r#"{"version":"x","type":"health","type":"health"}"#, "duplicate field `type`"),
        // inside a field
        (r#"{"type":"nested","ticket":{"id":"MQ-0"}}"#, "missing field `title`"),
        (r#"{"type":"nested","ticket":"x"}"#, "invalid type: string \"x\", expected struct Ticket"),
        (r#"{"type":"item","id":["A-1"]}"#, "invalid type: sequence, expected a string"),
        // not an object
        (r#""health""#, "invalid type: string \"health\", expected internally tagged enum House"),
        ("3", "invalid type: integer `3`, expected internally tagged enum House"),
        ("null", "invalid type: null, expected internally tagged enum House"),
        ("true", "invalid type: boolean `true`, expected internally tagged enum House"),
        ("[]", "missing field `type`"),
        (r#"["gone"]"#, "unknown variant `gone`, expected one of `health`, `item`, `ready`, `nested`, `error`"),
    ];
    for (json, words) in cases {
        assert_eq!(both_refuse::<HouseS, HouseF>(json), words, "{json}");
    }
}

twins! { renamed:
    #[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "kebab-case")]
    enum Renamed {
        TicketMoved {
            ticket_id: String,
            #[serde(rename = "to")]
            status: String,
            #[serde(alias = "byWhom", default)]
            by: Option<String>,
        },
        #[serde(rename = "gone!", alias = "removed")]
        Deleted { ids: Vec<String> },
        #[serde(skip_deserializing)]
        Hidden { x: u8 },
        Plain,
        #[serde(other)]
        Unknown,
    }
}

#[test]
fn renames_aliases_skips_and_other_read_as_serde_reads_them() {
    type S = renamed::serde_reads::Renamed;
    type F = renamed::type_first::Renamed;
    for json in [
        r#"{"kind":"ticketMoved","ticket-id":"A-1","to":"Ready"}"#,
        r#"{"kind":"ticketMoved","ticket-id":"A-1","to":"Ready","byWhom":"owner"}"#,
        r#"{"kind":"ticketMoved","ticket-id":"A-1","to":"Ready","by":"owner"}"#,
        r#"{"kind":"gone!","ids":["A-1","A-2"]}"#,
        r#"{"kind":"removed","ids":[]}"#,
        r#"{"kind":"plain"}"#,
        r#"{"kind":"whatever"}"#,
        r#"{"kind":"whatever","ids":["x"],"to":5}"#,
        r#"{"kind":"hidden","x":1}"#,
        r#"{"ids":["A-1"],"kind":"removed"}"#,
    ] {
        both_read::<S, F>(json);
    }
    for (json, words) in [
        (r#"{"kind":"ticketMoved","ticket_id":"A-1","to":"Ready"}"#, "missing field `ticket-id`"),
        (r#"{"kind":"ticketMoved","ticket-id":"A-1","to":"Ready","by":"a","byWhom":"b"}"#, "duplicate field `by`"),
        (r#"{"kind":"whatever","kind":"plain"}"#, "duplicate field `kind`"),
        (r#"{"kind":7}"#, "invalid type: integer `7`, expected variant identifier"),
        (r#"{"type":"plain"}"#, "missing field `kind`"),
    ] {
        assert_eq!(both_refuse::<S, F>(json), words, "{json}");
    }
}

#[derive(Debug, PartialEq, Deserialize)]
#[serde(tag = "kind")]
enum Inner {
    A { x: u8 },
    B,
}

/// A variant read `with` a function: `{ n }` read from its whole object, plus one.
fn plus_one<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u8, D::Error> {
    #[derive(Deserialize)]
    struct N {
        n: u8,
    }
    N::deserialize(d).map(|N { n }| n.saturating_add(1))
}

twins! { wrapped:
    #[serde(tag = "type", rename_all = "snake_case")]
    enum Wrapped {
        Ticket(Ticket),
        Map(BTreeMap<String, u8>),
        Inner(Inner),
        Skipped(#[serde(skip)] u8),
        #[serde(deserialize_with = "plus_one")]
        Counted { n: u8 },
        Flat {
            id: String,
            #[serde(flatten)]
            rest: BTreeMap<String, Value>,
        },
        Struct { a: u8 },
    }
}

#[test]
fn a_newtype_a_flattened_and_a_with_variant_read_as_serde_reads_them() {
    type S = wrapped::serde_reads::Wrapped;
    type F = wrapped::type_first::Wrapped;
    for json in [
        r#"{"type":"ticket","id":"MQ-0","title":"t","childCount":1,"severity":null}"#,
        r#"{"type":"ticket","title":"t","id":"MQ-0","childCount":1,"severity":"High","labels":["a"]}"#,
        r#"{"type":"map","a":1,"b":2}"#,
        r#"{"type":"map"}"#,
        r#"{"type":"inner","kind":"A","x":3}"#,
        r#"{"type":"inner","kind":"B"}"#,
        r#"{"type":"skipped","x":1}"#,
        r#"{"type":"counted","n":3}"#,
        r#"{"type":"flat","id":"x","b":1,"c":[1,{"d":null}]}"#,
        r#"{"type":"struct","a":1}"#,
        r#"{"a":1,"type":"struct"}"#,
        r#"{"id":"MQ-0","type":"ticket","title":"t","childCount":1,"severity":null}"#,
    ] {
        both_read::<S, F>(json);
    }
    for json in [
        r#"{"type":"ticket","id":"MQ-0"}"#,
        r#"{"type":"map","a":"x"}"#,
        r#"{"type":"map","type":"map"}"#,
        r#"{"type":"map","a":1,"type":"map"}"#,
        r#"{"type":"inner","kind":"C"}"#,
        r#"{"type":"counted"}"#,
        r#"{"type":"flat","b":1}"#,
        r#"{"type":"flat","id":"x","type":"flat"}"#,
        r#"{"type":"struct","a":300}"#,
    ] {
        both_refuse::<S, F>(json);
    }
}

twins! { strict:
    #[serde(tag = "type", deny_unknown_fields, expecting = "a strict event")]
    enum Strict {
        A { a: u8 },
        B,
    }
}

#[test]
fn deny_unknown_fields_and_expecting_read_as_serde_reads_them() {
    type S = strict::serde_reads::Strict;
    type F = strict::type_first::Strict;
    for json in [r#"{"type":"A","a":1}"#, r#"{"a":1,"type":"A"}"#, r#"{"type":"B"}"#, r#"{"type":"B","x":1}"#] {
        let _read_alike = alike::<S, F>(json);
    }
    assert_eq!(both_refuse::<S, F>(r#"{"type":"A","a":1,"b":2}"#), "unknown field `b`, expected `a`");
    assert_eq!(both_refuse::<S, F>(r#"{"type":"A","a":1,"type":"A"}"#), "duplicate field `type`");
    assert_eq!(both_refuse::<S, F>(r#""A""#), "invalid type: string \"A\", expected a strict event");
}

/// What only the input can carry, serde's buffer cannot hand on: a
/// `RawValue`, a map keyed by numbers. Read with its type first, a variant
/// gets them; with its type anywhere else it is serde's read, and refused as
/// serde refuses it.
#[test]
fn a_value_only_the_input_carries_reaches_a_variant_whose_type_comes_first() {
    #[derive(Debug, plexus_macros::Deserialize)]
    #[serde(tag = "type", rename_all = "snake_case")]
    enum Carried {
        Raw { body: Box<RawValue> },
        Counts { by_id: BTreeMap<u64, u8> },
    }
    #[derive(Debug, Deserialize)]
    #[serde(tag = "type", rename_all = "snake_case")]
    #[allow(dead_code)]
    enum Buffered {
        Raw { body: Box<RawValue> },
        Counts { by_id: BTreeMap<u64, u8> },
    }

    let Carried::Raw { body } = serde_json::from_str(r#"{"type":"raw","body":{"a":[1, 2]}}"#).unwrap() else {
        panic!("not raw")
    };
    assert_eq!(body.get(), r#"{"a":[1, 2]}"#);
    let Carried::Counts { by_id } = serde_json::from_str(r#"{"type":"counts","by_id":{"7":1,"9":2}}"#).unwrap() else {
        panic!("not counts")
    };
    assert_eq!(by_id, BTreeMap::from([(7, 1), (9, 2)]));

    for json in [r#"{"body":{"a":1},"type":"raw"}"#, r#"{"by_id":{"7":1},"type":"counts"}"#] {
        let refused = printed(serde_json::from_str::<Carried>(json));
        assert!(refused.is_err(), "{json}: {refused:?}");
        assert_eq!(refused, printed(serde_json::from_str::<Buffered>(json)), "{json}");
    }
    assert!(serde_json::from_str::<Buffered>(r#"{"type":"raw","body":{"a":1}}"#).is_err());
}

/// Where an object has two faults, serde names the second tag first (its
/// buffer reads the whole object before the variant), the read with the type
/// first the fault it meets first. One fault is named alike either way.
#[test]
fn of_two_faults_the_type_first_names_the_one_it_meets_first() {
    let json = r#"{"type":"health","version":7,"type":"ready"}"#;
    assert_eq!(printed(serde_json::from_str::<HouseS>(json)), Err("duplicate field `type`".into()));
    assert_eq!(printed(serde_json::from_str::<HouseF>(json)), Err("invalid type: integer `7`, expected a string".into()));
}

#[derive(Debug, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Status {
    Ready,
    Coding,
}

twins! { statused:
    #[serde(tag = "type", rename_all = "snake_case")]
    enum Statused {
        Moved { status: Status },
    }
}

/// A fault inside a field's value is named as the deserializer names it,
/// the words a struct holding that field gets, where serde's buffer had its
/// own: here an enum read from a number.
#[test]
fn a_fault_inside_a_value_is_named_as_a_struct_is_told() {
    #[derive(Debug, Deserialize)]
    #[allow(dead_code)]
    struct Holder {
        status: Status,
    }
    type S = statused::serde_reads::Statused;
    type F = statused::type_first::Statused;
    both_read::<S, F>(r#"{"type":"moved","status":"coding"}"#);
    both_refuse::<S, F>(r#"{"type":"moved","status":"gone"}"#);
    let json = r#"{"type":"moved","status":5}"#;
    assert_eq!(printed(serde_json::from_str::<S>(json)), Err("invalid type: integer `5`, expected string or map".into()));
    assert_eq!(printed(serde_json::from_str::<F>(json)), printed(serde_json::from_str::<Holder>(r#"{"status":5}"#)));
}
