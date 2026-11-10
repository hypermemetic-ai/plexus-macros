//! PLEXUS-59 · `#[wire]`: one attribute for a type that crosses the wire.

use plexus_macros::wire;

#[wire]
pub enum CountEvent {
    Tick { n: u32 },
    AllDone,
}

#[wire]
pub struct Point {
    x: i32,
    #[serde(default)]
    y: i32,
}

#[wire]
#[serde(untagged)]
pub enum Either {
    Number(u32),
    Word(String),
}

#[wire]
#[serde(tag = "kind", rename_all = "UPPERCASE")]
pub enum Own {
    Go { n: u32 },
}

#[test]
fn an_enum_is_tagged_type_in_snake_case() {
    assert_eq!(serde_json::to_string(&CountEvent::Tick { n: 2 }).unwrap(), r#"{"type":"tick","n":2}"#);
    assert_eq!(serde_json::to_string(&CountEvent::AllDone).unwrap(), r#"{"type":"all_done"}"#);
    let back: CountEvent = serde_json::from_str(r#"{"type":"tick","n":7}"#).unwrap();
    assert!(matches!(back.clone(), CountEvent::Tick { n: 7 }), "{back:?}");
}

#[test]
fn a_struct_is_read_written_and_described() {
    let p: Point = serde_json::from_str(r#"{"x":1}"#).unwrap();
    assert_eq!(serde_json::to_string(&p).unwrap(), r#"{"x":1,"y":0}"#);
    let schema = serde_json::to_value(schemars::schema_for!(Point)).unwrap();
    assert!(schema["properties"]["x"].is_object(), "{schema}");
    let schema = serde_json::to_string(&schemars::schema_for!(CountEvent)).unwrap();
    assert!(schema.contains("tick") && schema.contains("all_done"), "{schema}");
}

#[test]
fn an_enum_that_says_its_own_representation_keeps_it() {
    assert_eq!(serde_json::to_string(&Either::Number(3)).unwrap(), "3");
    assert_eq!(serde_json::to_string(&Either::Word("a".into())).unwrap(), r#""a""#);
    assert_eq!(serde_json::to_string(&Own::Go { n: 1 }).unwrap(), r#"{"kind":"GO","n":1}"#);
}
