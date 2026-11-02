//! The conformance fixture's item, error and progress types.
//!
//! Plain serde + schemars types with no dependency on the macro, so a test
//! that cannot compile the activation (plexus-core's, which reads this file
//! by sibling path) can still decode the expected JSON into them.
//!
//! Errors and progress steps follow the usual convention, an internally
//! tagged enum: `{"type": "not_found", "id": "missing"}`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// What `ping` answers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Pong {
    pub message: String,
}

/// What `lookup` finds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Thing {
    pub id: String,
    pub name: String,
}

/// Why `lookup` says no.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LookupError {
    NotFound { id: String },
}

/// One of `count`'s items.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Tick {
    pub n: u32,
}

/// One of `scan`'s rows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Row {
    pub key: String,
}

/// Why `scan` could not read a row. The scan says so and carries on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ScanError {
    Unreadable { key: String },
}

/// What `load` yields once it has loaded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Loaded {
    pub items: u32,
}

/// `load`'s typed progress.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LoadStep {
    Batch { done: u32, of: u32 },
}

/// Why `load` refuses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LoadError {
    TooMany { n: u32, max: u32 },
}

/// What `warm` yields once warm.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Warmed {
    pub entries: u32,
}
