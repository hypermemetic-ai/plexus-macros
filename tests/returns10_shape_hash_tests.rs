//! RETURNS-10: a method's hash covers its types' shape.
//!
//! One test per row of RETURNS-10's table. Each compares versions of one
//! activation, `Fx` (namespace `fx`, one method `fetch`), declared in
//! modules that differ by that row's change and nothing else. Every version
//! names its types `Item` (T), `Fail` (E) and `Step` (P): a schema carries a
//! type's bare name and never its module path (RETURNS-24), so only the
//! change itself can move the hash.
//!
//! The hash is `MethodSchema::content_hash` of the method's whole schema
//! (SHA-256 of its canonical JSON, 16 hex), applied by the macro's generated
//! `compute_method_schemas()` — what plexus-core's and the conformance
//! fixture's tests pin and recompute.

use plexus_core::plexus::{Activation, DynamicHub, PluginSchema};

/// What every version module imports.
mod prelude {
    pub use futures::{stream, Stream};
    pub use plexus_core::plexus::Progress;
    pub use schemars::JsonSchema;
    pub use serde::{Deserialize, Serialize};
}

// ---------------------------------------------------------------------------
// The fixture types, in versions: `v1` is the base, each other module changes
// one thing about it.
// ---------------------------------------------------------------------------

/// T.
mod item {
    pub mod v1 {
        use crate::prelude::*;
        /// An item.
        #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
        pub struct Item {
            pub id: String,
            pub size: u32,
            pub status: Status,
        }
        /// Where an item is.
        #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
        #[serde(rename_all = "snake_case")]
        pub enum Status {
            Open,
            Closed,
        }
    }
    pub mod field_added {
        use crate::prelude::*;
        pub use super::v1::Status;
        /// An item.
        #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
        pub struct Item {
            pub id: String,
            pub size: u32,
            pub status: Status,
            pub tag: Option<String>,
        }
    }
    pub mod field_renamed {
        use crate::prelude::*;
        pub use super::v1::Status;
        /// An item.
        #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
        pub struct Item {
            pub id: String,
            pub bytes: u32,
            pub status: Status,
        }
    }
    pub mod field_retyped {
        use crate::prelude::*;
        pub use super::v1::Status;
        /// An item.
        #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
        pub struct Item {
            pub id: String,
            pub size: u64,
            pub status: Status,
        }
    }
    /// A variant added to an enum nested inside T.
    pub mod nested_variant_added {
        use crate::prelude::*;
        /// An item.
        #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
        pub struct Item {
            pub id: String,
            pub size: u32,
            pub status: Status,
        }
        /// Where an item is.
        #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
        #[serde(rename_all = "snake_case")]
        pub enum Status {
            Open,
            Closed,
            Archived,
        }
    }
    /// A Rust field renamed with its wire name kept: the client cannot tell.
    pub mod serde_renamed {
        use crate::prelude::*;
        pub use super::v1::Status;
        /// An item.
        #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
        pub struct Item {
            pub id: String,
            #[serde(rename = "size")]
            pub length: u32,
            pub status: Status,
        }
    }
    /// Another type.
    pub mod other {
        use crate::prelude::*;
        /// A record.
        #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
        pub struct Record {
            pub name: String,
        }
    }
    /// `v1::Item`'s shape under another name.
    pub mod same_shape {
        use crate::prelude::*;
        pub use super::v1::Status;
        /// An item.
        #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
        pub struct Thing {
            pub id: String,
            pub size: u32,
            pub status: Status,
        }
    }
}

/// E.
mod fail {
    pub mod v1 {
        use crate::prelude::*;
        /// Why `fetch` says no.
        #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
        #[serde(tag = "type", rename_all = "snake_case")]
        pub enum Fail {
            NotFound { id: String },
            Locked { by: String },
        }
    }
    pub mod field_added {
        use crate::prelude::*;
        /// Why `fetch` says no.
        #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
        #[serde(tag = "type", rename_all = "snake_case")]
        pub enum Fail {
            NotFound { id: String, hint: Option<String> },
            Locked { by: String },
        }
    }
    pub mod field_renamed {
        use crate::prelude::*;
        /// Why `fetch` says no.
        #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
        #[serde(tag = "type", rename_all = "snake_case")]
        pub enum Fail {
            NotFound { key: String },
            Locked { by: String },
        }
    }
    pub mod field_retyped {
        use crate::prelude::*;
        /// Why `fetch` says no.
        #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
        #[serde(tag = "type", rename_all = "snake_case")]
        pub enum Fail {
            NotFound { id: u64 },
            Locked { by: String },
        }
    }
    pub mod other {
        use crate::prelude::*;
        /// A refusal.
        #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
        pub struct Refused {
            pub reason: String,
        }
    }
}

/// P.
mod step {
    pub mod v1 {
        use crate::prelude::*;
        /// How far `fetch` is.
        #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
        pub struct Step {
            pub done: u32,
            pub of: u32,
        }
    }
    pub mod field_added {
        use crate::prelude::*;
        /// How far `fetch` is.
        #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
        pub struct Step {
            pub done: u32,
            pub of: u32,
            pub label: String,
        }
    }
    pub mod field_renamed {
        use crate::prelude::*;
        /// How far `fetch` is.
        #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
        pub struct Step {
            pub done: u32,
            pub total: u32,
        }
    }
    pub mod field_retyped {
        use crate::prelude::*;
        /// How far `fetch` is.
        #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
        pub struct Step {
            pub done: f64,
            pub of: u32,
        }
    }
    pub mod other {
        use crate::prelude::*;
        /// A phase.
        #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
        pub struct Phase {
            pub name: String,
        }
    }
}

// ---------------------------------------------------------------------------
// The versions of `Fx`. Written out one module each: `#[activation]` does
// not expand inside a `macro_rules!` body (its generated jsonrpsee
// subscription mixes hygiene contexts), so no macro stamps them out.
// ---------------------------------------------------------------------------

/// The base: the full form over `v1`'s T, E and P.
mod base {
    #![allow(unused_variables, unused_imports)]
    use crate::prelude::*;
    use crate::{item::v1::Item as Item, fail::v1::Fail as Fail, step::v1::Step as Step};
    #[derive(Clone, Default)]
    pub struct Fx;
    #[plexus_macros::activation(namespace = "fx", version = "1.0.0", crate_path = "plexus_core")]
    impl Fx {
        /// Fetch an item, reporting progress.
        #[plexus_macros::method(params(id = "Item id"))]
        pub async fn fetch(&self, id: String) -> impl Stream<Item = Result<Progress<Item, Step>, Fail>> + Send + 'static {
            stream::empty()
        }
    }
}

/// The base's code again, in another module.
mod base_again {
    #![allow(unused_variables, unused_imports)]
    use crate::prelude::*;
    use crate::{item::v1::Item as Item, fail::v1::Fail as Fail, step::v1::Step as Step};
    #[derive(Clone, Default)]
    pub struct Fx;
    #[plexus_macros::activation(namespace = "fx", version = "1.0.0", crate_path = "plexus_core")]
    impl Fx {
        /// Fetch an item, reporting progress.
        #[plexus_macros::method(params(id = "Item id"))]
        pub async fn fetch(&self, id: String) -> impl Stream<Item = Result<Progress<Item, Step>, Fail>> + Send + 'static {
            stream::empty()
        }
    }
}

mod t_field_added {
    #![allow(unused_variables, unused_imports)]
    use crate::prelude::*;
    use crate::{item::field_added::Item as Item, fail::v1::Fail as Fail, step::v1::Step as Step};
    #[derive(Clone, Default)]
    pub struct Fx;
    #[plexus_macros::activation(namespace = "fx", version = "1.0.0", crate_path = "plexus_core")]
    impl Fx {
        /// Fetch an item, reporting progress.
        #[plexus_macros::method(params(id = "Item id"))]
        pub async fn fetch(&self, id: String) -> impl Stream<Item = Result<Progress<Item, Step>, Fail>> + Send + 'static {
            stream::empty()
        }
    }
}

mod t_field_renamed {
    #![allow(unused_variables, unused_imports)]
    use crate::prelude::*;
    use crate::{item::field_renamed::Item as Item, fail::v1::Fail as Fail, step::v1::Step as Step};
    #[derive(Clone, Default)]
    pub struct Fx;
    #[plexus_macros::activation(namespace = "fx", version = "1.0.0", crate_path = "plexus_core")]
    impl Fx {
        /// Fetch an item, reporting progress.
        #[plexus_macros::method(params(id = "Item id"))]
        pub async fn fetch(&self, id: String) -> impl Stream<Item = Result<Progress<Item, Step>, Fail>> + Send + 'static {
            stream::empty()
        }
    }
}

mod t_field_retyped {
    #![allow(unused_variables, unused_imports)]
    use crate::prelude::*;
    use crate::{item::field_retyped::Item as Item, fail::v1::Fail as Fail, step::v1::Step as Step};
    #[derive(Clone, Default)]
    pub struct Fx;
    #[plexus_macros::activation(namespace = "fx", version = "1.0.0", crate_path = "plexus_core")]
    impl Fx {
        /// Fetch an item, reporting progress.
        #[plexus_macros::method(params(id = "Item id"))]
        pub async fn fetch(&self, id: String) -> impl Stream<Item = Result<Progress<Item, Step>, Fail>> + Send + 'static {
            stream::empty()
        }
    }
}

mod t_nested_variant_added {
    #![allow(unused_variables, unused_imports)]
    use crate::prelude::*;
    use crate::{item::nested_variant_added::Item as Item, fail::v1::Fail as Fail, step::v1::Step as Step};
    #[derive(Clone, Default)]
    pub struct Fx;
    #[plexus_macros::activation(namespace = "fx", version = "1.0.0", crate_path = "plexus_core")]
    impl Fx {
        /// Fetch an item, reporting progress.
        #[plexus_macros::method(params(id = "Item id"))]
        pub async fn fetch(&self, id: String) -> impl Stream<Item = Result<Progress<Item, Step>, Fail>> + Send + 'static {
            stream::empty()
        }
    }
}

mod t_serde_renamed {
    #![allow(unused_variables, unused_imports)]
    use crate::prelude::*;
    use crate::{item::serde_renamed::Item as Item, fail::v1::Fail as Fail, step::v1::Step as Step};
    #[derive(Clone, Default)]
    pub struct Fx;
    #[plexus_macros::activation(namespace = "fx", version = "1.0.0", crate_path = "plexus_core")]
    impl Fx {
        /// Fetch an item, reporting progress.
        #[plexus_macros::method(params(id = "Item id"))]
        pub async fn fetch(&self, id: String) -> impl Stream<Item = Result<Progress<Item, Step>, Fail>> + Send + 'static {
            stream::empty()
        }
    }
}

mod e_field_added {
    #![allow(unused_variables, unused_imports)]
    use crate::prelude::*;
    use crate::{item::v1::Item as Item, fail::field_added::Fail as Fail, step::v1::Step as Step};
    #[derive(Clone, Default)]
    pub struct Fx;
    #[plexus_macros::activation(namespace = "fx", version = "1.0.0", crate_path = "plexus_core")]
    impl Fx {
        /// Fetch an item, reporting progress.
        #[plexus_macros::method(params(id = "Item id"))]
        pub async fn fetch(&self, id: String) -> impl Stream<Item = Result<Progress<Item, Step>, Fail>> + Send + 'static {
            stream::empty()
        }
    }
}

mod e_field_renamed {
    #![allow(unused_variables, unused_imports)]
    use crate::prelude::*;
    use crate::{item::v1::Item as Item, fail::field_renamed::Fail as Fail, step::v1::Step as Step};
    #[derive(Clone, Default)]
    pub struct Fx;
    #[plexus_macros::activation(namespace = "fx", version = "1.0.0", crate_path = "plexus_core")]
    impl Fx {
        /// Fetch an item, reporting progress.
        #[plexus_macros::method(params(id = "Item id"))]
        pub async fn fetch(&self, id: String) -> impl Stream<Item = Result<Progress<Item, Step>, Fail>> + Send + 'static {
            stream::empty()
        }
    }
}

mod e_field_retyped {
    #![allow(unused_variables, unused_imports)]
    use crate::prelude::*;
    use crate::{item::v1::Item as Item, fail::field_retyped::Fail as Fail, step::v1::Step as Step};
    #[derive(Clone, Default)]
    pub struct Fx;
    #[plexus_macros::activation(namespace = "fx", version = "1.0.0", crate_path = "plexus_core")]
    impl Fx {
        /// Fetch an item, reporting progress.
        #[plexus_macros::method(params(id = "Item id"))]
        pub async fn fetch(&self, id: String) -> impl Stream<Item = Result<Progress<Item, Step>, Fail>> + Send + 'static {
            stream::empty()
        }
    }
}

mod p_field_added {
    #![allow(unused_variables, unused_imports)]
    use crate::prelude::*;
    use crate::{item::v1::Item as Item, fail::v1::Fail as Fail, step::field_added::Step as Step};
    #[derive(Clone, Default)]
    pub struct Fx;
    #[plexus_macros::activation(namespace = "fx", version = "1.0.0", crate_path = "plexus_core")]
    impl Fx {
        /// Fetch an item, reporting progress.
        #[plexus_macros::method(params(id = "Item id"))]
        pub async fn fetch(&self, id: String) -> impl Stream<Item = Result<Progress<Item, Step>, Fail>> + Send + 'static {
            stream::empty()
        }
    }
}

mod p_field_renamed {
    #![allow(unused_variables, unused_imports)]
    use crate::prelude::*;
    use crate::{item::v1::Item as Item, fail::v1::Fail as Fail, step::field_renamed::Step as Step};
    #[derive(Clone, Default)]
    pub struct Fx;
    #[plexus_macros::activation(namespace = "fx", version = "1.0.0", crate_path = "plexus_core")]
    impl Fx {
        /// Fetch an item, reporting progress.
        #[plexus_macros::method(params(id = "Item id"))]
        pub async fn fetch(&self, id: String) -> impl Stream<Item = Result<Progress<Item, Step>, Fail>> + Send + 'static {
            stream::empty()
        }
    }
}

mod p_field_retyped {
    #![allow(unused_variables, unused_imports)]
    use crate::prelude::*;
    use crate::{item::v1::Item as Item, fail::v1::Fail as Fail, step::field_retyped::Step as Step};
    #[derive(Clone, Default)]
    pub struct Fx;
    #[plexus_macros::activation(namespace = "fx", version = "1.0.0", crate_path = "plexus_core")]
    impl Fx {
        /// Fetch an item, reporting progress.
        #[plexus_macros::method(params(id = "Item id"))]
        pub async fn fetch(&self, id: String) -> impl Stream<Item = Result<Progress<Item, Step>, Fail>> + Send + 'static {
            stream::empty()
        }
    }
}

mod t_replaced {
    #![allow(unused_variables, unused_imports)]
    use crate::prelude::*;
    use crate::{item::other::Record as Item, fail::v1::Fail as Fail, step::v1::Step as Step};
    #[derive(Clone, Default)]
    pub struct Fx;
    #[plexus_macros::activation(namespace = "fx", version = "1.0.0", crate_path = "plexus_core")]
    impl Fx {
        /// Fetch an item, reporting progress.
        #[plexus_macros::method(params(id = "Item id"))]
        pub async fn fetch(&self, id: String) -> impl Stream<Item = Result<Progress<Item, Step>, Fail>> + Send + 'static {
            stream::empty()
        }
    }
}

mod t_same_shape_renamed {
    #![allow(unused_variables, unused_imports)]
    use crate::prelude::*;
    use crate::{item::same_shape::Thing as Item, fail::v1::Fail as Fail, step::v1::Step as Step};
    #[derive(Clone, Default)]
    pub struct Fx;
    #[plexus_macros::activation(namespace = "fx", version = "1.0.0", crate_path = "plexus_core")]
    impl Fx {
        /// Fetch an item, reporting progress.
        #[plexus_macros::method(params(id = "Item id"))]
        pub async fn fetch(&self, id: String) -> impl Stream<Item = Result<Progress<Item, Step>, Fail>> + Send + 'static {
            stream::empty()
        }
    }
}

mod e_replaced {
    #![allow(unused_variables, unused_imports)]
    use crate::prelude::*;
    use crate::{item::v1::Item as Item, fail::other::Refused as Fail, step::v1::Step as Step};
    #[derive(Clone, Default)]
    pub struct Fx;
    #[plexus_macros::activation(namespace = "fx", version = "1.0.0", crate_path = "plexus_core")]
    impl Fx {
        /// Fetch an item, reporting progress.
        #[plexus_macros::method(params(id = "Item id"))]
        pub async fn fetch(&self, id: String) -> impl Stream<Item = Result<Progress<Item, Step>, Fail>> + Send + 'static {
            stream::empty()
        }
    }
}

mod p_replaced {
    #![allow(unused_variables, unused_imports)]
    use crate::prelude::*;
    use crate::{item::v1::Item as Item, fail::v1::Fail as Fail, step::other::Phase as Step};
    #[derive(Clone, Default)]
    pub struct Fx;
    #[plexus_macros::activation(namespace = "fx", version = "1.0.0", crate_path = "plexus_core")]
    impl Fx {
        /// Fetch an item, reporting progress.
        #[plexus_macros::method(params(id = "Item id"))]
        pub async fn fetch(&self, id: String) -> impl Stream<Item = Result<Progress<Item, Step>, Fail>> + Send + 'static {
            stream::empty()
        }
    }
}

/// One item, fallible, no progress: `-> Result<Item, Fail>`.
mod result_one {
    #![allow(unused_variables, unused_imports)]
    use crate::prelude::*;
    use crate::{item::v1::Item, fail::v1::Fail};
    #[derive(Clone, Default)]
    pub struct Fx;
    #[plexus_macros::activation(namespace = "fx", version = "1.0.0", crate_path = "plexus_core")]
    impl Fx {
        /// Fetch an item, reporting progress.
        #[plexus_macros::method(params(id = "Item id"))]
        pub async fn fetch(&self, id: String) -> Result<Item, Fail> {
            unimplemented!()
        }
    }
}

/// `result_one` as a stream, and the base without its `Progress`.
mod result_many {
    #![allow(unused_variables, unused_imports)]
    use crate::prelude::*;
    use crate::{item::v1::Item, fail::v1::Fail};
    #[derive(Clone, Default)]
    pub struct Fx;
    #[plexus_macros::activation(namespace = "fx", version = "1.0.0", crate_path = "plexus_core")]
    impl Fx {
        /// Fetch an item, reporting progress.
        #[plexus_macros::method(params(id = "Item id"))]
        pub async fn fetch(&self, id: String) -> impl Stream<Item = Result<Item, Fail>> + Send + 'static {
            stream::empty()
        }
    }
}

/// The base without its `Result`.
mod progress_many {
    #![allow(unused_variables, unused_imports)]
    use crate::prelude::*;
    use crate::{item::v1::Item, step::v1::Step};
    #[derive(Clone, Default)]
    pub struct Fx;
    #[plexus_macros::activation(namespace = "fx", version = "1.0.0", crate_path = "plexus_core")]
    impl Fx {
        /// Fetch an item, reporting progress.
        #[plexus_macros::method(params(id = "Item id"))]
        pub async fn fetch(&self, id: String) -> impl Stream<Item = Progress<Item, Step>> + Send + 'static {
            stream::empty()
        }
    }
}

/// The base without either wrapper.
mod plain_many {
    #![allow(unused_variables, unused_imports)]
    use crate::prelude::*;
    use crate::{item::v1::Item};
    #[derive(Clone, Default)]
    pub struct Fx;
    #[plexus_macros::activation(namespace = "fx", version = "1.0.0", crate_path = "plexus_core")]
    impl Fx {
        /// Fetch an item, reporting progress.
        #[plexus_macros::method(params(id = "Item id"))]
        pub async fn fetch(&self, id: String) -> impl Stream<Item = Item> + Send + 'static {
            stream::empty()
        }
    }
}

mod param_added {
    #![allow(unused_variables, unused_imports)]
    use crate::prelude::*;
    use crate::{item::v1::Item as Item, fail::v1::Fail as Fail, step::v1::Step as Step};
    #[derive(Clone, Default)]
    pub struct Fx;
    #[plexus_macros::activation(namespace = "fx", version = "1.0.0", crate_path = "plexus_core")]
    impl Fx {
        /// Fetch an item, reporting progress.
        #[plexus_macros::method(params(id = "Item id", deep = "Fetch nested items"))]
        pub async fn fetch(&self, id: String, deep: bool) -> impl Stream<Item = Result<Progress<Item, Step>, Fail>> + Send + 'static {
            stream::empty()
        }
    }
}

mod param_renamed {
    #![allow(unused_variables, unused_imports)]
    use crate::prelude::*;
    use crate::{item::v1::Item as Item, fail::v1::Fail as Fail, step::v1::Step as Step};
    #[derive(Clone, Default)]
    pub struct Fx;
    #[plexus_macros::activation(namespace = "fx", version = "1.0.0", crate_path = "plexus_core")]
    impl Fx {
        /// Fetch an item, reporting progress.
        #[plexus_macros::method(params(key = "Item id"))]
        pub async fn fetch(&self, key: String) -> impl Stream<Item = Result<Progress<Item, Step>, Fail>> + Send + 'static {
            stream::empty()
        }
    }
}

mod param_retyped {
    #![allow(unused_variables, unused_imports)]
    use crate::prelude::*;
    use crate::{item::v1::Item as Item, fail::v1::Fail as Fail, step::v1::Step as Step};
    #[derive(Clone, Default)]
    pub struct Fx;
    #[plexus_macros::activation(namespace = "fx", version = "1.0.0", crate_path = "plexus_core")]
    impl Fx {
        /// Fetch an item, reporting progress.
        #[plexus_macros::method(params(id = "Item id"))]
        pub async fn fetch(&self, id: u64) -> impl Stream<Item = Result<Progress<Item, Step>, Fail>> + Send + 'static {
            stream::empty()
        }
    }
}

mod param_doc_edited {
    #![allow(unused_variables, unused_imports)]
    use crate::prelude::*;
    use crate::{item::v1::Item as Item, fail::v1::Fail as Fail, step::v1::Step as Step};
    #[derive(Clone, Default)]
    pub struct Fx;
    #[plexus_macros::activation(namespace = "fx", version = "1.0.0", crate_path = "plexus_core")]
    impl Fx {
        /// Fetch an item, reporting progress.
        #[plexus_macros::method(params(id = "The item's id"))]
        pub async fn fetch(&self, id: String) -> impl Stream<Item = Result<Progress<Item, Step>, Fail>> + Send + 'static {
            stream::empty()
        }
    }
}

mod description_edited {
    #![allow(unused_variables, unused_imports)]
    use crate::prelude::*;
    use crate::{item::v1::Item as Item, fail::v1::Fail as Fail, step::v1::Step as Step};
    #[derive(Clone, Default)]
    pub struct Fx;
    #[plexus_macros::activation(namespace = "fx", version = "1.0.0", crate_path = "plexus_core")]
    impl Fx {
        /// Fetch one item, reporting progress.
        #[plexus_macros::method(params(id = "Item id"))]
        pub async fn fetch(&self, id: String) -> impl Stream<Item = Result<Progress<Item, Step>, Fail>> + Send + 'static {
            stream::empty()
        }
    }
}


// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn schema<A: Activation>(a: A) -> PluginSchema {
    a.plugin_schema()
}

/// `fetch`'s hash in a version's schema.
fn fetch_hash(schema: &PluginSchema) -> String {
    schema
        .methods
        .iter()
        .find(|m| m.name == "fetch")
        .expect("fx has fetch")
        .hash
        .clone()
}

fn is_hash16(h: &str) -> bool {
    h.len() == 16 && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Every version's `fetch` hash differs from the base's and from each other's.
fn assert_all_move(base: &str, versions: &[(&str, String)]) {
    let mut seen = std::collections::HashMap::from([(base.to_string(), "base")]);
    for (what, h) in versions {
        assert!(is_hash16(h), "{what}: 16 lowercase hex: {h}");
        if let Some(prev) = seen.insert(h.clone(), what) {
            panic!("{what}: hash {h} did not move (same as {prev})");
        }
    }
}

// ---------------------------------------------------------------------------
// The table's rows.
// ---------------------------------------------------------------------------

/// The base's `fetch` hash, pinned. Its schema is fixed by this file, so a
/// clean build anywhere — another machine, rustc, target dir, or
/// `serde_json/preserve_order` switched on by feature unification — must
/// produce exactly this (`scripts/hash-stability.sh` runs it twice).
const BASE_FETCH_HASH: &str = "9a1a5aeb37ca8e66";

/// Row: unchanged code, rebuilt (same or another machine) → unchanged.
#[test]
fn unchanged_code_keeps_its_hash() {
    let a = schema(base::Fx);
    let b = schema(base_again::Fx);
    assert!(is_hash16(&fetch_hash(&a)), "{}", fetch_hash(&a));
    // Same code in another module: the module path reaches no schema.
    assert_eq!(fetch_hash(&a), fetch_hash(&b));
    assert_eq!((a.self_hash.clone(), a.hash.clone()), (b.self_hash.clone(), b.hash.clone()));
    // Computed again in the same process: the same.
    assert_eq!(fetch_hash(&a), fetch_hash(&schema(base::Fx)));
    // Rebuilt: the pinned value.
    assert_eq!(fetch_hash(&a), BASE_FETCH_HASH, "the base's hash moved — a clean build must not move it");
}

/// Row: a field added / renamed / retyped inside T, E or P → changes (the
/// pass branch). T's nested enum gaining a variant counts as a field change.
#[test]
fn a_field_change_inside_t_e_or_p_changes_the_hash() {
    let base = fetch_hash(&schema(base::Fx));
    assert_all_move(
        &base,
        &[
            ("T field added", fetch_hash(&schema(t_field_added::Fx))),
            ("T field renamed", fetch_hash(&schema(t_field_renamed::Fx))),
            ("T field retyped", fetch_hash(&schema(t_field_retyped::Fx))),
            ("T nested enum variant added", fetch_hash(&schema(t_nested_variant_added::Fx))),
            ("E field added", fetch_hash(&schema(e_field_added::Fx))),
            ("E field renamed", fetch_hash(&schema(e_field_renamed::Fx))),
            ("E field retyped", fetch_hash(&schema(e_field_retyped::Fx))),
            ("P field added", fetch_hash(&schema(p_field_added::Fx))),
            ("P field renamed", fetch_hash(&schema(p_field_renamed::Fx))),
            ("P field retyped", fetch_hash(&schema(p_field_retyped::Fx))),
        ],
    );
}

/// Row: T, E or P replaced by another type → changes — even one of the same
/// shape under another name (its `title` and `$defs` key move).
#[test]
fn t_e_or_p_replaced_changes_the_hash() {
    let base = fetch_hash(&schema(base::Fx));
    assert_all_move(
        &base,
        &[
            ("T replaced", fetch_hash(&schema(t_replaced::Fx))),
            ("T replaced by its shape renamed", fetch_hash(&schema(t_same_shape_renamed::Fx))),
            ("E replaced", fetch_hash(&schema(e_replaced::Fx))),
            ("P replaced", fetch_hash(&schema(p_replaced::Fx))),
        ],
    );
}

/// Row: cardinality one ↔ many → changes.
#[test]
fn cardinality_one_or_many_changes_the_hash() {
    let one = fetch_hash(&schema(result_one::Fx));
    let many = fetch_hash(&schema(result_many::Fx));
    assert!(is_hash16(&one) && is_hash16(&many));
    assert_ne!(one, many, "Result<Item, Fail> vs impl Stream<Item = Result<Item, Fail>>");
}

/// Row: a `Result` or `Progress` wrapper added or removed → changes.
#[test]
fn a_result_or_progress_wrapper_changes_the_hash() {
    let full = fetch_hash(&schema(base::Fx)); // Result<Progress<Item, Step>, Fail>
    assert_all_move(
        &full,
        &[
            ("Progress removed: Result<Item, Fail>", fetch_hash(&schema(result_many::Fx))),
            ("Result removed: Progress<Item, Step>", fetch_hash(&schema(progress_many::Fx))),
            ("both removed: Item", fetch_hash(&schema(plain_many::Fx))),
        ],
    );
}

/// Row: a param added, renamed or retyped, or the description edited →
/// changes. (A param's doc is part of the params schema, so it moves too.)
#[test]
fn a_param_or_description_change_changes_the_hash() {
    let base = fetch_hash(&schema(base::Fx));
    assert_all_move(
        &base,
        &[
            ("param added", fetch_hash(&schema(param_added::Fx))),
            ("param renamed", fetch_hash(&schema(param_renamed::Fx))),
            ("param retyped", fetch_hash(&schema(param_retyped::Fx))),
            ("param doc edited", fetch_hash(&schema(param_doc_edited::Fx))),
            ("description edited", fetch_hash(&schema(description_edited::Fx))),
        ],
    );
}

/// Beyond the table (RETURNS-24): a Rust field renamed with its wire name
/// kept by `#[serde(rename)]` does not move the hash — the schema, and so
/// every generated client, is the same.
#[test]
fn a_rust_only_rename_keeps_the_hash() {
    assert_eq!(fetch_hash(&schema(base::Fx)), fetch_hash(&schema(t_serde_renamed::Fx)));
}

/// The roll-up: a method's hash moving moves its plugin's `self_hash` and
/// `hash`, and the hash of a hub the plugin is registered in — the root
/// hash a backend serves as `plexus_hash`, synapse reads as `irHash` and
/// synapse-cc locks.
#[test]
fn a_field_change_moves_the_plugin_and_root_hashes() {
    let before = schema(base::Fx);
    let after = schema(t_field_added::Fx);
    assert_ne!(fetch_hash(&before), fetch_hash(&after));
    assert_ne!(before.self_hash, after.self_hash, "self_hash");
    assert_ne!(before.hash, after.hash, "plugin hash");
    // The other methods (`schema`) did not move.
    let others = |s: &PluginSchema| {
        s.methods.iter().filter(|m| m.name != "fetch").map(|m| m.hash.clone()).collect::<Vec<_>>()
    };
    assert_eq!(others(&before), others(&after));

    let root_before = DynamicHub::new("root").register(base::Fx);
    let root_after = DynamicHub::new("root").register(t_field_added::Fx);
    let root_again = DynamicHub::new("root").register(base_again::Fx);
    assert_ne!(root_before.compute_hash(), root_after.compute_hash(), "root hash");
    assert_eq!(root_before.compute_hash(), root_again.compute_hash(), "unchanged root");
    assert!(is_hash16(&root_before.compute_hash()));
}
