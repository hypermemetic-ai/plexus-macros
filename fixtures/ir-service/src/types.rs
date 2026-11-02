//! The types the fixture's activations share — between methods of one
//! activation and between activations — and the two the IR reader refuses.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One item, as every activation names it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Item {
    /// Its id.
    pub id: String,
    /// What it is called, when it is called anything.
    pub label: Option<String>,
    /// Its colour.
    pub color: Color,
    /// Its tags, in order.
    pub tags: Vec<String>,
}

/// A colour: a string enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Color {
    Red,
    Green,
    Blue,
}

/// What happens to items: a tagged enum, one variant a newtype over a struct.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ItemEvent {
    /// An item was added.
    Added { item: Item },
    /// An item was removed.
    Removed { id: String },
    /// An item was moved: the move itself.
    Moved(Move),
}

/// Where an item went.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Move {
    pub id: String,
    pub from: u32,
    pub to: u32,
}

/// A page of items.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Page {
    pub items: Vec<Item>,
    /// Where the next page starts; none at the end.
    pub next: Option<String>,
    /// Items by colour.
    pub counts: BTreeMap<String, u32>,
    /// The page's span: first and last index.
    pub span: (u32, u32),
}

/// Why an item cannot be had.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Missing {
    NotFound { id: String },
    Gone { id: String, since: u64 },
}

/// A step of a long load.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Step {
    Fetched { done: u32, of: u32 },
}

/// A bounded name: its bounds are carried as refinements.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Name {
    #[schemars(length(min = 1, max = 64))]
    pub value: String,
    #[schemars(range(min = 1, max = 9))]
    pub rank: u8,
}

/// Refused by the IR reader: a map flattened beside properties has no IR
/// form.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Odd {
    pub name: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, String>,
}

/// Refused by the IR reader: an untagged union of a string and a number.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Choice {
    Word(String),
    Number(u32),
}
