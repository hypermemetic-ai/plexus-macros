//! RETURNS-4: `Progress<T, P = Note>`, `Note`, `Frame<I, P>`.

use plexus_client::{Frame, Note, Progress};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[test]
fn note_wire_shape() {
    let n = Note::new("indexing").with_percentage(50.0);
    assert_eq!(
        serde_json::to_value(&n).unwrap(),
        json!({"message": "indexing", "percentage": 50.0})
    );
    assert_eq!(
        serde_json::from_value::<Note>(json!({"message": "indexing", "percentage": 50.0})).unwrap(),
        n
    );
}

#[test]
fn note_omits_percentage_when_none() {
    let n = Note::new("starting");
    assert_eq!(
        serde_json::to_value(&n).unwrap(),
        json!({"message": "starting"})
    );
    assert_eq!(
        serde_json::from_value::<Note>(json!({"message": "starting"})).unwrap(),
        n
    );
    assert_eq!(
        serde_json::from_value::<Note>(json!({"message": "starting", "percentage": null})).unwrap(),
        n
    );
    assert!(serde_json::from_value::<Note>(json!({"percentage": 1.0})).is_err());
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ImportStep {
    Chunk { done: usize, of: usize },
}

#[derive(Debug, Clone, PartialEq)]
struct ImportSummary {
    rows: usize,
}

#[derive(Debug, Clone, PartialEq)]
enum ImportError {
    Load { chunk: usize },
}

/// The Intended Usage: progress sits inside the Result, `P` defaults to
/// `Note`.
#[test]
fn an_author_item_type() {
    type ImportItem = Result<Progress<ImportSummary, ImportStep>, ImportError>;
    let items: Vec<ImportItem> = vec![
        Ok(Progress::Update(ImportStep::Chunk { done: 3, of: 10 })),
        Ok(Progress::Item(ImportSummary { rows: 412 })),
        Err(ImportError::Load { chunk: 4 }),
    ];
    assert!(items[0].as_ref().unwrap().is_update());
    assert!(items[1].as_ref().unwrap().is_item());
    assert_eq!(
        items[1].clone().unwrap().into_item(),
        Some(ImportSummary { rows: 412 })
    );
    assert_eq!(items[0].clone().unwrap().into_item(), None);

    // `Progress<T>` is the untyped case.
    let untyped: Progress<u32> = Progress::Update(Note::new("warming up"));
    assert_eq!(untyped.into_update(), Some(Note::new("warming up")));
}

/// What the server sends as `Progress` arrives as `Frame`.
#[test]
fn progress_maps_onto_frame() {
    let f: Frame<u32, ImportStep> = Progress::Update(ImportStep::Chunk { done: 1, of: 2 }).into();
    assert_eq!(f, Frame::Progress(ImportStep::Chunk { done: 1, of: 2 }));
    assert!(f.is_progress() && !f.is_item());
    assert_eq!(
        f.clone().into_progress(),
        Some(ImportStep::Chunk { done: 1, of: 2 })
    );
    assert_eq!(f.into_item(), None);

    let f: Frame<u32, Note> = Progress::Item(7).into();
    assert_eq!(f, Frame::Item(7));
    assert_eq!(f.item(), Some(&7));
    assert_eq!(f.into_item(), Some(7));
}

/// A receiver's match over an opted-in stream's frames.
#[test]
fn a_receiver_matches_frames() {
    let frames: Vec<Frame<Result<u32, String>, Note>> = vec![
        Frame::Progress(Note::new("half").with_percentage(50.0)),
        Frame::Item(Ok(1)),
        Frame::Item(Err("no".into())),
    ];
    let mut seen = Vec::new();
    for f in frames {
        match f {
            Frame::Progress(Note { percentage, .. }) => seen.push(format!("{percentage:?}")),
            Frame::Item(Ok(n)) => seen.push(format!("ok {n}")),
            Frame::Item(Err(e)) => seen.push(format!("err {e}")),
        }
    }
    assert_eq!(seen, ["Some(50.0)", "ok 1", "err no"]);
}
