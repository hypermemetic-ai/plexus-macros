//! The tree fixture's item and error types: plain serde + schemars types,
//! errors internally tagged (`{"type": "frost", "at": 2}`).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// What `tree.ping` answers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Pong {
    pub message: String,
}

/// One of `tree.count`'s items.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Tick {
    pub n: u32,
}

/// One of `tree.branch.grow`'s buds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Bud {
    pub bud: u32,
}

/// Why a bud did not grow. The branch says so and grows on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GrowError {
    Frost { at: u32 },
}

/// One of `tree.branch.leaf.fall`'s leaves.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Fell {
    pub leaf: u32,
}

/// One of `tree.<name>.watch`'s sightings: the instance's own name, so a
/// test can hold every event's `name` to the instance that sent it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Seen {
    pub seen: String,
    pub n: u32,
}
