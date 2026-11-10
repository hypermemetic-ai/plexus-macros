//! PLEXUS-59 · `#[derive(PlexusWire)]`: a type that crosses the wire.

use plexus_macros::PlexusWire;

#[derive(Debug, Clone, PlexusWire)]
pub enum CountEvent {
    Tick { n: u32 },
    AllDone,
}

#[derive(Debug, Clone, PlexusWire)]
pub struct Point {
    x: i32,
    #[serde(default)]
    y: i32,
}

#[derive(Debug, Clone, PlexusWire)]
#[serde(untagged)]
pub enum Either {
    Number(u32),
    Word(String),
}

#[derive(Debug, Clone, PlexusWire)]
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
    assert_eq!(schema["title"], "Point", "{schema}");
    let schema = serde_json::to_string(&schemars::schema_for!(CountEvent)).unwrap();
    assert!(!schema.contains("PlexusWire"), "the twin is named nowhere: {schema}");
    assert!(schema.contains("tick") && schema.contains("all_done"), "{schema}");
}

#[test]
fn an_enum_that_says_its_own_representation_keeps_it() {
    assert_eq!(serde_json::to_string(&Either::Number(3)).unwrap(), "3");
    assert_eq!(serde_json::to_string(&Either::Word("a".into())).unwrap(), r#""a""#);
    assert_eq!(serde_json::to_string(&Own::Go { n: 1 }).unwrap(), r#"{"kind":"GO","n":1}"#);
}

/// A wire type inside another, and private fields.
mod inner {
    use plexus_macros::PlexusWire;

    #[derive(Debug, Clone, PartialEq, PlexusWire)]
    pub struct Line {
        from: super::Point2,
        /// Where it ends.
        to: Option<super::Point2>,
    }

    pub fn line() -> Line {
        Line { from: super::Point2 { x: 1 }, to: None }
    }
}

#[derive(Debug, Clone, PartialEq, PlexusWire)]
pub struct Point2 {
    x: i32,
}

#[test]
fn a_wire_type_holds_another() {
    let l = inner::line();
    let s = serde_json::to_string(&l).unwrap();
    assert_eq!(s, r#"{"from":{"x":1},"to":null}"#);
    assert_eq!(serde_json::from_str::<inner::Line>(&s).unwrap(), l);
    let schema = serde_json::to_string(&schemars::schema_for!(inner::Line)).unwrap();
    assert!(schema.contains("Point2") && schema.contains("Where it ends."), "{schema}");
}
