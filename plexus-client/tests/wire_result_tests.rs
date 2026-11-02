//! RETURNS-4 acceptance 1 and 2: the `{ok}|{err}` encoding of a Result
//! item, standalone and inside an adjacently-tagged enum (the `{Ns}Event`
//! aggregate's shape).

use plexus_client::wire_result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Ticket {
    id: String,
    title: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum GetError {
    NotFound { id: String },
    Archived,
}

/// The Intended Usage shape: a newtype around a Result item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Wrapped(#[serde(with = "wire_result")] Result<Ticket, GetError>);

fn ticket() -> Ticket {
    Ticket {
        id: "MNG-2".into(),
        title: "precise returns".into(),
    }
}

fn round_trips<T>(value: T, wire: Value)
where
    T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug,
{
    assert_eq!(
        serde_json::to_value(&value).unwrap(),
        wire,
        "serialize {value:?}"
    );
    assert_eq!(
        serde_json::from_value::<T>(wire.clone()).unwrap(),
        value,
        "deserialize {wire}"
    );
    let text = serde_json::to_string(&value).unwrap();
    assert_eq!(
        serde_json::from_str::<T>(&text).unwrap(),
        value,
        "text {text}"
    );
}

#[test]
fn ok_is_an_ok_object() {
    round_trips(
        Wrapped(Ok(ticket())),
        json!({"ok": {"id": "MNG-2", "title": "precise returns"}}),
    );
}

#[test]
fn err_is_an_err_object() {
    // The literal from RETURNS-1's wire example.
    round_trips(
        Wrapped(Err(GetError::NotFound {
            id: "MNG-999".into(),
        })),
        json!({"err": {"type": "not_found", "id": "MNG-999"}}),
    );
    round_trips(
        Wrapped(Err(GetError::Archived)),
        json!({"err": {"type": "archived"}}),
    );
}

#[test]
fn unit_and_option_payloads_keep_their_key() {
    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Unit(#[serde(with = "wire_result")] Result<(), String>);
    round_trips(Unit(Ok(())), json!({"ok": null}));
    round_trips(Unit(Err("no".into())), json!({"err": "no"}));

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Maybe(#[serde(with = "wire_result")] Result<Option<u32>, Option<u32>>);
    round_trips(Maybe(Ok(None)), json!({"ok": null}));
    round_trips(Maybe(Err(None)), json!({"err": null}));
    round_trips(Maybe(Ok(Some(3))), json!({"ok": 3}));
}

#[test]
fn as_a_struct_field() {
    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Row {
        seq: u64,
        #[serde(with = "wire_result")]
        item: Result<Ticket, GetError>,
    }
    round_trips(
        Row {
            seq: 4,
            item: Ok(ticket()),
        },
        json!({"seq": 4, "item": {"ok": {"id": "MNG-2", "title": "precise returns"}}}),
    );
}

/// The functions are usable directly, with a `serde_json::Value` as the
/// (de)serializer — how a decoder reads a data frame's `content`.
#[test]
fn direct_calls_on_a_value() {
    let content = json!({"err": {"type": "archived"}});
    let r: Result<Ticket, GetError> = wire_result::deserialize(content.clone()).unwrap();
    assert_eq!(r, Err(GetError::Archived));
    assert_eq!(
        wire_result::serialize(&r, serde_json::value::Serializer).unwrap(),
        content
    );
}

/// Any third shape is a decode error whose message names both keys.
#[test]
fn a_third_shape_names_both_keys() {
    for wire in [
        json!({"value": {"id": "MNG-2", "title": "t"}}),
        json!({"Ok": {"id": "MNG-2", "title": "t"}}),
        json!({}),
        json!({"ok": {"id": "MNG-2", "title": "t"}, "err": {"type": "archived"}}),
        json!({"err": {"type": "archived"}, "extra": 1}),
        json!({"id": "MNG-2", "title": "t"}),
        json!("ok"),
        json!(7),
        json!(null),
        json!(true),
        json!([{"ok": 1}]),
    ] {
        // From a Value (the buffered path) …
        let e = serde_json::from_value::<Wrapped>(wire.clone())
            .expect_err(&format!("{wire} must not decode"))
            .to_string();
        assert!(e.contains("`ok`") && e.contains("`err`"), "{wire}: {e}");
        // … and from text (the streaming path).
        let e = serde_json::from_str::<Wrapped>(&wire.to_string())
            .expect_err(&format!("{wire} must not decode"))
            .to_string();
        assert!(e.contains("`ok`") && e.contains("`err`"), "{wire}: {e}");
    }
}

/// The payload under the right key still has to decode as its own type.
#[test]
fn a_bad_payload_under_a_good_key_is_an_error() {
    assert!(serde_json::from_value::<Wrapped>(json!({"ok": {"id": 3}})).is_err());
    assert!(serde_json::from_value::<Wrapped>(json!({"err": {"type": "nope"}})).is_err());
}

// ── acceptance 2: inside an adjacently-tagged enum ─────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ListItem {
    Row { id: String },
    Page { next: Option<String> },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Health {
    version: String,
}

/// The `{Ns}Event` aggregate's shape: one variant per method, keyed by wire
/// name, carrying the method's item — a Result item through `wire_result`,
/// a plain item as itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "method", content = "event", rename_all = "snake_case")]
enum TicketsEvent {
    Health(Health),
    Get(#[serde(with = "plexus_client::wire_result")] Result<Ticket, GetError>),
    List(#[serde(with = "plexus_client::wire_result")] Result<ListItem, GetError>),
}

#[test]
fn newtype_variant_in_an_adjacently_tagged_enum_round_trips() {
    round_trips(
        TicketsEvent::Get(Ok(ticket())),
        json!({"method": "get", "event": {"ok": {"id": "MNG-2", "title": "precise returns"}}}),
    );
    round_trips(
        TicketsEvent::Get(Err(GetError::NotFound {
            id: "MNG-999".into(),
        })),
        json!({"method": "get", "event": {"err": {"type": "not_found", "id": "MNG-999"}}}),
    );
    round_trips(
        TicketsEvent::List(Ok(ListItem::Page { next: None })),
        json!({"method": "list", "event": {"ok": {"type": "page", "next": null}}}),
    );
    round_trips(
        TicketsEvent::Health(Health {
            version: "0.7.0".into(),
        }),
        json!({"method": "health", "event": {"version": "0.7.0"}}),
    );
}

/// serde buffers the content when it precedes the tag; `wire_result` must
/// decode from that buffered form too.
#[test]
fn content_before_tag_still_decodes() {
    let text = r#"{"event": {"err": {"type": "archived"}}, "method": "get"}"#;
    assert_eq!(
        serde_json::from_str::<TicketsEvent>(text).unwrap(),
        TicketsEvent::Get(Err(GetError::Archived))
    );
    let text = r#"{"event": {"ok": {"type": "row", "id": "MNG-2"}}, "method": "list"}"#;
    assert_eq!(
        serde_json::from_str::<TicketsEvent>(text).unwrap(),
        TicketsEvent::List(Ok(ListItem::Row { id: "MNG-2".into() }))
    );
}

#[test]
fn a_third_shape_inside_the_aggregate_names_both_keys() {
    for text in [
        r#"{"method": "get", "event": {"id": "MNG-2", "title": "t"}}"#,
        r#"{"event": {"id": "MNG-2", "title": "t"}, "method": "get"}"#,
    ] {
        let e = serde_json::from_str::<TicketsEvent>(text)
            .expect_err("bare item is not a Result item")
            .to_string();
        assert!(e.contains("`ok`") && e.contains("`err`"), "{text}: {e}");
    }
}
