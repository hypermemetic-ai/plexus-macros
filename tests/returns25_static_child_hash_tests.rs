//! RETURNS-25: a static `#[child]` namespace reaches its parent's hash.
//!
//! The macro's synthesized `plugin_children` used to fill every
//! `ChildSummary.hash` with `""`, so a change inside a static child moved no
//! ancestor's hash — not the root's, which synapse reads as `irHash` and
//! synapse-cc locks. Now each summary carries the child's own schema hash
//! (`Activation::plugin_schema(&child).hash`), which rolls up into the
//! parent's `children_hash` and `hash`.
//!
//! The fixture has hyperforge's shape: a top hub (`forge`) with two static
//! children — `repo`, itself a hub over the leaf `images`, and `build`, a
//! `const fn` accessor — served under a DynamicHub root. Two versions differ
//! only in a field of `images.list`'s item, two static levels down.

use plexus_core::plexus::{Activation, ChildSummary, DynamicHub, PluginSchema};

mod prelude {
    pub use futures::{stream, Stream};
    pub use schemars::JsonSchema;
    pub use serde::{Deserialize, Serialize};
}

/// `images.list`'s item, in two versions.
mod item {
    pub mod v1 {
        use crate::prelude::*;
        /// A container image.
        #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
        pub struct Item {
            pub tag: String,
        }
    }
    pub mod field_added {
        use crate::prelude::*;
        /// A container image.
        #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
        pub struct Item {
            pub tag: String,
            pub digest: Option<String>,
        }
    }
}

// Written out per version: `#[activation]` does not expand inside a
// `macro_rules!` body (see tests/returns10_shape_hash_tests.rs).
/// Version 1: `images.list` yields `item::v1::Item`.
mod v1 {
    #![allow(dead_code, unused_imports)]
    use crate::prelude::*;
    use crate::item::v1::Item;

    /// A leaf two static levels below the root hub: its one method's item
    /// is the type that changes between versions.
    #[derive(Clone, Default)]
    pub struct Images;

    #[plexus_macros::activation(namespace = "images", version = "1.0.0", crate_path = "plexus_core")]
    impl Images {
        /// List images.
        #[plexus_macros::method]
        pub async fn list(&self) -> impl Stream<Item = Item> + Send + 'static {
            stream::empty()
        }
    }

    /// A static leaf beside `repo`, the same in every version.
    #[derive(Clone, Default)]
    pub struct Builds;

    #[plexus_macros::activation(namespace = "build", version = "1.0.0", crate_path = "plexus_core")]
    impl Builds {
        /// Build status.
        #[plexus_macros::method]
        pub async fn status(&self) -> String {
            "idle".into()
        }
    }

    /// A hub with one static child (hyperforge's `RepoHub` → `images`).
    #[derive(Clone, Default)]
    pub struct Repo;

    #[plexus_macros::activation(namespace = "repo", version = "1.0.0", crate_path = "plexus_core")]
    impl Repo {
        /// The repo's name.
        #[plexus_macros::method]
        pub async fn name(&self) -> String {
            "r".into()
        }

        /// Container image operations.
        #[plexus_macros::child]
        fn images(&self) -> Images {
            Images
        }
    }

    /// The top hub (hyperforge's `HyperforgeHub`): two static children, one
    /// of them a `const fn` with an attribute of its own, as hyperforge's
    /// `build` is.
    #[derive(Clone, Default)]
    pub struct Forge;

    #[plexus_macros::activation(namespace = "forge", version = "1.0.0", crate_path = "plexus_core")]
    impl Forge {
        /// The forge's status.
        #[plexus_macros::method]
        pub async fn status(&self) -> String {
            "ok".into()
        }

        /// Single-repo operations.
        #[plexus_macros::child]
        fn repo(&self) -> Repo {
            Repo
        }

        /// Build orchestration.
        #[plexus_macros::child]
        #[allow(clippy::unused_self)]
        const fn build(&self) -> Builds {
            Builds
        }
    }
}

/// Version 2: the same code, but `Item` has gained a field.
mod v2 {
    #![allow(dead_code, unused_imports)]
    use crate::prelude::*;
    use crate::item::field_added::Item;

    /// A leaf two static levels below the root hub: its one method's item
    /// is the type that changes between versions.
    #[derive(Clone, Default)]
    pub struct Images;

    #[plexus_macros::activation(namespace = "images", version = "1.0.0", crate_path = "plexus_core")]
    impl Images {
        /// List images.
        #[plexus_macros::method]
        pub async fn list(&self) -> impl Stream<Item = Item> + Send + 'static {
            stream::empty()
        }
    }

    /// A static leaf beside `repo`, the same in every version.
    #[derive(Clone, Default)]
    pub struct Builds;

    #[plexus_macros::activation(namespace = "build", version = "1.0.0", crate_path = "plexus_core")]
    impl Builds {
        /// Build status.
        #[plexus_macros::method]
        pub async fn status(&self) -> String {
            "idle".into()
        }
    }

    /// A hub with one static child (hyperforge's `RepoHub` → `images`).
    #[derive(Clone, Default)]
    pub struct Repo;

    #[plexus_macros::activation(namespace = "repo", version = "1.0.0", crate_path = "plexus_core")]
    impl Repo {
        /// The repo's name.
        #[plexus_macros::method]
        pub async fn name(&self) -> String {
            "r".into()
        }

        /// Container image operations.
        #[plexus_macros::child]
        fn images(&self) -> Images {
            Images
        }
    }

    /// The top hub (hyperforge's `HyperforgeHub`): two static children, one
    /// of them a `const fn` with an attribute of its own, as hyperforge's
    /// `build` is.
    #[derive(Clone, Default)]
    pub struct Forge;

    #[plexus_macros::activation(namespace = "forge", version = "1.0.0", crate_path = "plexus_core")]
    impl Forge {
        /// The forge's status.
        #[plexus_macros::method]
        pub async fn status(&self) -> String {
            "ok".into()
        }

        /// Single-repo operations.
        #[plexus_macros::child]
        fn repo(&self) -> Repo {
            Repo
        }

        /// Build orchestration.
        #[plexus_macros::child]
        #[allow(clippy::unused_self)]
        const fn build(&self) -> Builds {
            Builds
        }
    }
}

fn is_hash16(h: &str) -> bool {
    h.len() == 16 && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// The `ChildSummary` list a schema carries (the deprecated side-table
/// `PluginSchema::children`, where the roll-up reads child hashes from).
#[allow(deprecated)]
fn children(schema: &PluginSchema) -> Vec<ChildSummary> {
    schema.children.clone().expect("a hub lists its children")
}

fn child_hash(schema: &PluginSchema, name: &str) -> String {
    children(schema)
        .into_iter()
        .find(|c| c.namespace == name)
        .unwrap_or_else(|| panic!("no child {name}"))
        .hash
}

/// Each synthesized summary carries its child's own schema hash — never
/// `""` — including the `const fn` child, at both static levels.
#[test]
fn a_static_childs_summary_carries_its_schema_hash() {
    let forge = v1::Forge.plugin_schema();
    let repo = v1::Repo.plugin_schema();
    for (name, expected) in [
        ("repo", repo.hash.clone()),
        ("build", v1::Builds.plugin_schema().hash),
    ] {
        let h = child_hash(&forge, name);
        assert!(is_hash16(&h), "forge.{name}: 16 lowercase hex, got {h:?}");
        assert_eq!(h, expected, "forge.{name}: the child's own plugin hash");
    }
    assert_eq!(child_hash(&repo, "images"), v1::Images.plugin_schema().hash);
    assert_eq!(
        v1::Forge.plugin_children().iter().map(|c| c.hash.clone()).collect::<Vec<_>>(),
        children(&forge).iter().map(|c| c.hash.clone()).collect::<Vec<_>>(),
        "plugin_schema() lists what plugin_children() synthesizes"
    );
    // The roll-up: children_hash is the hash of the child hashes.
    let rebuilt = PluginSchema::hub("forge", "1.0.0", "", forge.methods.clone(), children(&forge));
    assert_eq!(rebuilt.children_hash, forge.children_hash);
}

/// Acceptance 1: changing a field in the child's method item changes the
/// child's hash, the parent's hash and the root hash — here through two
/// static levels (`images` → `repo` → `forge`) and a DynamicHub root.
#[test]
fn a_field_change_in_a_static_child_moves_every_ancestors_hash() {
    let (images1, images2) = (v1::Images.plugin_schema(), v2::Images.plugin_schema());
    let (repo1, repo2) = (v1::Repo.plugin_schema(), v2::Repo.plugin_schema());
    let (forge1, forge2) = (v1::Forge.plugin_schema(), v2::Forge.plugin_schema());

    // The child: its method's hash, and so its own.
    let list = |s: &PluginSchema| s.methods.iter().find(|m| m.name == "list").unwrap().hash.clone();
    assert_ne!(list(&images1), list(&images2), "images.list's hash");
    assert_ne!(images1.hash, images2.hash, "images' hash");

    // Its parent: its own methods did not change, its children did.
    assert_eq!(repo1.self_hash, repo2.self_hash, "repo's own methods are the same");
    assert_ne!(child_hash(&repo1, "images"), child_hash(&repo2, "images"));
    assert_ne!(repo1.children_hash, repo2.children_hash, "repo's children_hash");
    assert_ne!(repo1.hash, repo2.hash, "repo's hash");

    // Its grandparent, the top hub: likewise; its other child did not move.
    assert_eq!(forge1.self_hash, forge2.self_hash, "forge's own methods are the same");
    assert_eq!(child_hash(&forge1, "build"), child_hash(&forge2, "build"), "build did not change");
    assert_ne!(child_hash(&forge1, "repo"), child_hash(&forge2, "repo"));
    assert_ne!(forge1.children_hash, forge2.children_hash, "forge's children_hash");
    assert_ne!(forge1.hash, forge2.hash, "forge's hash");

    // The root a backend serves (`plexus_hash`, synapse's `irHash`).
    let root1 = DynamicHub::new("root").register(v1::Forge);
    let root2 = DynamicHub::new("root").register(v2::Forge);
    assert_ne!(root1.compute_hash(), root2.compute_hash(), "root hash");
    assert_eq!(
        root1.compute_hash(),
        DynamicHub::new("root").register(v1::Forge).compute_hash(),
        "an unchanged root keeps its hash"
    );
}
