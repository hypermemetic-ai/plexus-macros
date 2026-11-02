//! RETURNS-40: a service of several activations, for the IR the service
//! writes (`<service>.ir`, plexus-core) and the IR synapse builds from the
//! same service's raw schemas (`buildIRFromSchemas`) — held equal.
//!
//! | activation | at | what it brings |
//! |---|---|---|
//! | `alpha` | `alpha` | a fallible method, a stream, a struct param, a static child |
//! | `gamma` | `alpha.gamma` | a nested child; a type the reader refuses (`Odd`, `Choice`) |
//! | `beta` | `beta` | the same types as `alpha` (shared between activations), typed progress, an array return |
//! | `clash` | `clash` (its own service) | two types titled `Thing` with different shapes: refused |
//!
//! `expected/` holds what synapse makes of it: `schemas.json` (every
//! `<path>.schema` answer, by path — `""` the root), `ir.json` (the whole
//! service), `ir.alpha.json` (the `alpha` subtree), and for the clash
//! service `clash.schemas.json` and `clash.error.txt`. synapse's
//! `ir3-test` rewrites the IR files from the schemas
//! (`SYNAPSE_GOLDEN_ACCEPT=1`); `tests/schemas.rs` rewrites the schemas
//! from the served service (`IR_FIXTURE_ACCEPT=1`).

pub mod types;
pub use types::*;

use async_stream::stream;
use futures::Stream;
use plexus_client::Progress;
use plexus_core::plexus::DynamicHub;

/// The service's name: its root namespace, and synapse's backend.
pub const SERVICE: &str = "irsvc";

/// The clash service's name.
pub const CLASH_SERVICE: &str = "clashsvc";

fn item(id: &str) -> Item {
    Item { id: id.into(), label: None, color: Color::Red, tags: vec![] }
}

/// Items, by id.
#[derive(Debug, Clone, Copy, Default)]
pub struct AlphaActivation;

#[plexus_macros::activation(
    namespace = "alpha",
    version = "1.0.0",
    description = "Items, by id",
    crate_path = "plexus_core",
    roles(viewer = ["get", "list", "watch"], editor = [viewer, "paint", "rename"]),
)]
impl AlphaActivation {
    /// The item with this id.
    ///
    /// An id no item has is `not_found`; one that had an item, `gone`.
    #[plexus_macros::method(target = "id", params(id = "Item id"))]
    pub async fn get(&self, id: String) -> Result<Item, Missing> {
        if id == "missing" {
            return Err(Missing::NotFound { id });
        }
        Ok(item(&id))
    }

    /// A page of items.
    #[plexus_macros::method(params(cursor = "Where to start"))]
    pub async fn list(&self, cursor: Option<String>) -> Page {
        let _ = cursor;
        Page { items: vec![item("a1")], next: None, counts: Default::default(), span: (0, 0) }
    }

    /// Every change to the items.
    #[plexus_macros::method]
    pub async fn watch(&self) -> impl Stream<Item = ItemEvent> + Send + 'static {
        stream! { yield ItemEvent::Removed { id: "a1".into() }; }
    }

    /// Paint an item.
    #[plexus_macros::method(params(id = "Item id", color = "The new colour"))]
    pub async fn paint(&self, id: String, color: Color) -> Item {
        Item { color, ..item(&id) }
    }

    /// The items' names.
    #[plexus_macros::method(params(name = "A name"))]
    pub async fn rename(&self, name: Name) -> Name {
        name
    }

    /// Item tools, one level down.
    #[plexus_macros::child]
    fn gamma(&self) -> GammaActivation {
        GammaActivation
    }
}

/// Item tools.
#[derive(Debug, Clone, Copy, Default)]
pub struct GammaActivation;

#[plexus_macros::activation(namespace = "gamma", version = "1.0.0", description = "Item tools", crate_path = "plexus_core")]
impl GammaActivation {
    /// The item back.
    #[plexus_macros::method(params(item = "An item"))]
    pub async fn echo(&self, item: Item) -> Item {
        item
    }

    /// A type the IR reader refuses: a map flattened beside a field.
    #[plexus_macros::method]
    pub async fn odd(&self) -> Odd {
        Odd { name: "o".into(), extra: Default::default() }
    }

    /// A type the IR reader refuses: an untagged union.
    #[plexus_macros::method(params(choice = "A word or a number"))]
    pub async fn pick(&self, choice: Choice) -> Choice {
        choice
    }
}

/// Items, by query.
#[derive(Debug, Clone, Copy, Default)]
pub struct BetaActivation;

#[plexus_macros::activation(namespace = "beta", version = "1.0.0", description = "Items, by query", crate_path = "plexus_core")]
impl BetaActivation {
    /// The items matching a query.
    #[plexus_macros::method(params(query = "What to match"))]
    pub async fn find(&self, query: String) -> Vec<Item> {
        vec![item(&query)]
    }

    /// Every change to the items, as `alpha.watch` says them.
    #[plexus_macros::method]
    pub async fn events(&self) -> impl Stream<Item = ItemEvent> + Send + 'static {
        stream! { yield ItemEvent::Added { item: item("b1") }; }
    }

    /// Load `n` pages, saying each step.
    #[plexus_macros::method(params(n = "How many pages"))]
    pub async fn load(&self, n: u32) -> impl Stream<Item = Progress<Page, Step>> + Send + 'static {
        stream! {
            yield Progress::Update(Step::Fetched { done: 1, of: n });
            yield Progress::Item(Page { items: vec![], next: None, counts: Default::default(), span: (0, 0) });
        }
    }

    /// The item with this id, or why not.
    #[plexus_macros::method(params(id = "Item id"))]
    pub async fn get(&self, id: String) -> Result<Item, Missing> {
        Err(Missing::Gone { id, since: 1 })
    }
}

/// Two types titled `Thing`: one in each module.
pub mod things {
    pub mod one {
        use schemars::JsonSchema;
        use serde::{Deserialize, Serialize};
        /// A thing, the first way.
        #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
        pub struct Thing {
            pub id: String,
        }
    }
    pub mod two {
        use schemars::JsonSchema;
        use serde::{Deserialize, Serialize};
        /// A thing, the second way.
        #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
        pub struct Thing {
            pub id: u64,
            pub weight: u32,
        }
    }
}

/// Two shapes under one title.
#[derive(Debug, Clone, Copy, Default)]
pub struct ClashActivation;

#[plexus_macros::activation(namespace = "clash", version = "1.0.0", description = "Two shapes under one title", crate_path = "plexus_core")]
impl ClashActivation {
    /// A thing, the first way.
    #[plexus_macros::method]
    pub async fn one(&self) -> things::one::Thing {
        things::one::Thing { id: "1".into() }
    }

    /// A thing, the second way.
    #[plexus_macros::method]
    pub async fn two(&self) -> things::two::Thing {
        things::two::Thing { id: 2, weight: 3 }
    }
}

/// The service: `alpha` (with `gamma` below it) and `beta`.
pub fn service() -> DynamicHub {
    DynamicHub::new(SERVICE).register(AlphaActivation).register(BetaActivation)
}

/// The clash service: `clash` alone.
pub fn clash_service() -> DynamicHub {
    DynamicHub::new(CLASH_SERVICE).register(ClashActivation)
}
