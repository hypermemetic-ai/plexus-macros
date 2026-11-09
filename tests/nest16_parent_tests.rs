//! NEST-16 · `#[activation(parent = ShopClient)]`: a graft's needs, inferred
//! from the call chains on its injected `parent`.
//!
//! - a method of the parent (`parent.items()`), whatever follows it on the
//!   stream (`.collect()`), is `shop.items`, with the shape hash it was built
//!   against;
//! - a chain through static children (`parent.ui().slots().list()`) is the
//!   method at its end, `shop.ui.slots.list`;
//! - `uses(…)` adds what the code cannot show, and lets `parent` leave a
//!   chain;
//! - `parent.host()` (the standard upward interface) is no need;
//! - a dynamic child's accessor is named as written (`shop.planet`), so the
//!   graft refuses it, naming it, until `uses` says the real path;
//! - the parent is not on the wire: not a param in the schema, not in the
//!   client.
//!
//! The trybuild cases: `parent` escaping a chain without `uses` is a compile
//! error at that use.

use futures::stream::{Stream, StreamExt};
use plexus_core::plexus::peer::Interface;
use plexus_core::plexus::{Activation, Need, Parent};

pub mod shop {
    use super::*;

    #[derive(Clone)]
    pub struct Slots;

    /// The slots.
    #[plexus_macros::activation]
    impl Slots {
        /// Every slot.
        pub async fn list(&self) -> impl Stream<Item = String> + Send + 'static {
            futures::stream::iter(["nav".to_string()])
        }
    }

    #[derive(Clone)]
    pub struct Ui;

    /// The UI.
    #[plexus_macros::activation]
    impl Ui {
        /// Its theme.
        pub async fn theme(&self) -> impl Stream<Item = String> + Send + 'static {
            futures::stream::iter(["dark".to_string()])
        }
        #[plexus_macros::child]
        fn slots(&self) -> Slots {
            Slots
        }
    }

    #[derive(Clone)]
    pub struct Planet;

    /// A planet.
    #[plexus_macros::activation]
    impl Planet {
        /// Its mass.
        pub async fn mass(&self) -> impl Stream<Item = u64> + Send + 'static {
            futures::stream::iter([1])
        }
    }

    #[derive(Clone)]
    pub struct Shop;

    /// The shop.
    #[plexus_macros::activation]
    impl Shop {
        /// What is on the shelves.
        pub async fn items(&self) -> impl Stream<Item = String> + Send + 'static {
            futures::stream::iter(["flour".to_string()])
        }
        /// Its till.
        pub async fn till(&self) -> impl Stream<Item = u64> + Send + 'static {
            futures::stream::iter([42])
        }
        #[plexus_macros::child]
        fn ui(&self) -> Ui {
            Ui
        }
        #[plexus_macros::child]
        fn planet(&self, name: &str) -> Option<Planet> {
            (name == "earth").then_some(Planet)
        }
    }
}

use shop::ShopClient;

#[derive(Clone)]
pub struct Restock;

/// Knows its parent.
#[plexus_macros::activation(parent = ShopClient)]
impl Restock {
    /// The items, collected.
    pub async fn low(&self, parent: Parent<ShopClient>, below: u32) -> impl Stream<Item = String> + Send + 'static {
        let items: Vec<String> = parent.items().filter_map(|i| async move { i.ok() }).collect().await;
        let _ = parent.host().whoami().await;
        futures::stream::iter(items.into_iter().take(below as usize))
    }
    /// A child's method, three links down.
    pub async fn slots(&self, parent: Parent<ShopClient>) -> impl Stream<Item = String> + Send + 'static {
        let got: Vec<String> = parent.ui().slots().list().filter_map(|i| async move { i.ok() }).collect().await;
        futures::stream::iter(got)
    }
    /// Through a dynamic child: not inferable, named as written.
    pub async fn mass(&self, parent: Parent<ShopClient>) -> impl Stream<Item = u64> + Send + 'static {
        let got: Vec<u64> = parent.planet("earth").mass().filter_map(|i| async move { i.ok() }).collect().await;
        futures::stream::iter(got)
    }
    /// The parent passed on: allowed by `uses`.
    #[plexus_macros::method(uses("shop.till"))]
    pub async fn count(&self, parent: Parent<ShopClient>) -> impl Stream<Item = u64> + Send + 'static {
        let n = tally(&parent).await;
        futures::stream::iter([n])
    }
}

async fn tally(p: &Parent<ShopClient<plexus_core::plexus::ParentTransport>>) -> u64 {
    p.till().filter_map(|i| async move { i.ok() }).collect::<Vec<_>>().await.into_iter().sum()
}

#[test]
fn needs_are_inferred_from_the_chains() {
    let needs = Restock::__plexus_needs();
    let paths: Vec<&str> = needs.iter().map(|n| n.method.as_str()).collect();
    assert_eq!(paths, ["shop.items", "shop.planet", "shop.till", "shop.ui.slots.list"], "{needs:?}");
    let hash = |schemas: Vec<plexus_core::plexus::MethodSchema>, name: &str| {
        Interface::method_hash(schemas.iter().find(|m| m.name == name).unwrap())
    };
    let shop = shop::Shop.plugin_schema().methods;
    let slots = shop::Slots.plugin_schema().methods;
    assert_eq!(needs[0], Need::new("shop.items", hash(shop.clone(), "items")));
    assert_eq!(needs[1], Need::new("shop.planet", ""), "a dynamic child's accessor: named as written");
    assert_eq!(needs[2], Need::new("shop.till", hash(shop, "till")), "uses(…) resolved to its hash");
    assert_eq!(needs[3], Need::new("shop.ui.slots.list", hash(slots, "list")));
    assert_eq!(Restock.plugin_schema().needs, Some(needs.clone()));
    assert_eq!(Restock.needs(), Some(needs));
    assert_eq!(shop::Shop.plugin_schema().needs, None);
}

#[test]
fn the_parent_is_not_on_the_wire() {
    let schema = Restock.plugin_schema();
    let low = schema.methods.iter().find(|m| m.name == "low").unwrap();
    let params = serde_json::to_value(&low.params).unwrap();
    assert!(params["properties"].get("below").is_some(), "{params}");
    assert!(params["properties"].get("parent").is_none(), "{params}");
    // The client takes `below` alone.
    let client = RestockClient::new(plexus_client::MemoryTransport::new());
    let _ = client.low(3);
}

#[test]
fn parent_compile() {
    let t = trybuild::TestCases::new();
    t.pass("tests/compile/nest16_parent_chains.rs");
    t.compile_fail("tests/compile/nest16_parent_escapes.rs");
    t.compile_fail("tests/compile/nest16_parent_rebound.rs");
    t.compile_fail("tests/compile/nest16_parent_cloned.rs");
    t.compile_fail("tests/compile/nest16_parent_without_attribute.rs");
}
