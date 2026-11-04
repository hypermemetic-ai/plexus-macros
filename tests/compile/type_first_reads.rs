//! A house event derives `plexus_macros::Deserialize` beside serde's
//! `Serialize`, and reads what it writes, its type first or not.

#[derive(Debug, PartialEq, serde::Serialize, plexus_macros::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EchoEvent {
    Said { text: String, n: u64 },
    Ready,
    Error { message: String },
}

fn main() {
    let said = EchoEvent::Said { text: "hi".into(), n: 1 };
    let text = serde_json::to_string(&said).unwrap();
    assert_eq!(text, r#"{"type":"said","text":"hi","n":1}"#);
    assert_eq!(serde_json::from_str::<EchoEvent>(&text).unwrap(), said);
    assert_eq!(serde_json::from_str::<EchoEvent>(r#"{"n":1,"text":"hi","type":"said"}"#).unwrap(), said);
    assert_eq!(serde_json::from_str::<EchoEvent>(r#"{"type":"ready","x":[1]}"#).unwrap(), EchoEvent::Ready);
}
