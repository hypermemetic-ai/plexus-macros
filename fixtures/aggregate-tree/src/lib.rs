//! RETURNS-46: the aggregate up the tree. Four activations, three levels
//! deep, one of them dynamic:
//!
//! | route | activation | namespace | aggregate | what it is |
//! |---|---|---|---|---|
//! | `tree` | [`TreeActivation`] | `tree` | `TreeEvent` | the root: `ping`, `count` |
//! | `tree.branch` | [`BranchActivation`] | `branch` | `BranchEvent` | a static child: `grow` (a Result item) |
//! | `tree.branch.leaf` | [`LeafActivation`] | `foliage` | `FoliageEvent` | a static child of the static child: `fall` |
//! | `tree.<name>` | [`ItemActivation`] | `item` | `ItemEvent` | a dynamic child, instances `a` and `b`: `watch` |
//!
//! The leaf's namespace is not its stem on purpose: its client is
//! `LeafClient` (the stem), its aggregate `FoliageEvent` (the namespace), so
//! a parent names a child's aggregate through its client, never by guessing.
//!
//! The derived aggregates nest (`expected/aggregate.<level>.jsonl`):
//!
//! ```text
//! {"method":"branch","event":{"method":"leaf","event":{"method":"fall","event":{"leaf":1}}}}
//! {"method":"item","event":{"name":"a","event":{"method":"watch","event":{"seen":"a","n":1}}}}
//! ```
//!
//! Every body is deterministic: same params, same items, same order.

pub mod types;
pub use types::*;

use async_stream::stream;
use futures::Stream;

/// The dynamic child's instances; any other name is not found.
pub const ITEMS: [&str; 2] = ["a", "b"];

/// The bud `grow` cannot grow.
const FROSTED: u32 = 2;

/// The root.
#[derive(Debug, Clone, Copy, Default)]
pub struct TreeActivation;

#[plexus_macros::activation(description = "The root of the aggregate tree", crate_path = "plexus_core")]
impl TreeActivation {
    /// Always answers pong.
    #[plexus_macros::method]
    pub async fn ping(&self) -> Pong {
        Pong { message: "pong".into() }
    }

    /// Ticks from 1 up to `to`.
    #[plexus_macros::method(params(to = "Last tick"))]
    pub async fn count(&self, to: u32) -> impl Stream<Item = Tick> + Send + 'static {
        stream! {
            for n in 1..=to {
                yield Tick { n };
            }
        }
    }

    /// The branch: a static child.
    #[plexus_macros::child]
    pub fn branch(&self) -> BranchActivation {
        BranchActivation
    }

    /// An item by name — `a` or `b`: a dynamic child.
    #[plexus_macros::child]
    pub fn item(&self, name: &str) -> Option<ItemActivation> {
        ITEMS.contains(&name).then(|| ItemActivation { name: name.to_string() })
    }
}

/// The static child, with a static child of its own.
#[derive(Debug, Clone, Copy, Default)]
pub struct BranchActivation;

#[plexus_macros::activation(description = "A branch of the tree", crate_path = "plexus_core")]
impl BranchActivation {
    /// Grows `n` buds; bud 2 is frosted, and the branch grows on.
    #[plexus_macros::method(params(n = "Buds to grow"))]
    pub async fn grow(&self, n: u32) -> impl Stream<Item = Result<Bud, GrowError>> + Send + 'static {
        stream! {
            for bud in 1..=n {
                if bud == FROSTED {
                    yield Err(GrowError::Frost { at: bud });
                } else {
                    yield Ok(Bud { bud });
                }
            }
        }
    }

    /// The leaf: the static child's static child.
    #[plexus_macros::child]
    pub fn leaf(&self) -> LeafActivation {
        LeafActivation
    }
}

/// The grandchild. Its namespace, `foliage`, is not its stem, `leaf`.
#[derive(Debug, Clone, Copy, Default)]
pub struct LeafActivation;

#[plexus_macros::activation(namespace = "foliage", description = "A leaf of the branch", crate_path = "plexus_core")]
impl LeafActivation {
    /// `n` leaves fall, one by one.
    #[plexus_macros::method(params(n = "Leaves to fall"))]
    pub async fn fall(&self, n: u32) -> impl Stream<Item = Fell> + Send + 'static {
        stream! {
            for leaf in 1..=n {
                yield Fell { leaf };
            }
        }
    }
}

/// The dynamic child: one instance per name in [`ITEMS`].
#[derive(Debug, Clone, Default)]
pub struct ItemActivation {
    name: String,
}

#[plexus_macros::activation(description = "An item of the tree, by name", crate_path = "plexus_core")]
impl ItemActivation {
    /// This item's own name, `n` times.
    #[plexus_macros::method(params(n = "Sightings"))]
    pub async fn watch(&self, n: u32) -> impl Stream<Item = Seen> + Send + 'static {
        let seen = self.name.clone();
        stream! {
            for n in 1..=n {
                yield Seen { seen: seen.clone(), n };
            }
        }
    }
}

// The activations' traits are always emitted, and their clients implement
// them (RETURNS-36), so a client failure must become each method's item.
macro_rules! from_client_error {
    ($($ty:ty => |$e:ident| $make:expr;)*) => {$(
        impl plexus_client::FromClientError for $ty {
            fn from_client_error($e: plexus_client::ClientError) -> Self {
                $make
            }
        }
    )*};
}

from_client_error! {
    Pong => |e| Pong { message: e.to_string() };
    Tick => |_e| Tick { n: 0 };
    GrowError => |_e| GrowError::Frost { at: 0 };
    Fell => |_e| Fell { leaf: 0 };
    Seen => |e| Seen { seen: e.to_string(), n: 0 };
}
