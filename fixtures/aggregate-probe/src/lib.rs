//! RETURNS-46's review probes, as a fixture (the reviewer's `probe` crate,
//! ported): the shapes the aggregate up the tree must survive.
//!
//! | route | activation | what it probes |
//! |---|---|---|
//! | `root` | [`RootActivation`] | `tick`; two static children of one type (`left`, `right`); a dynamic child `org` |
//! | `root.left`, `root.right` | [`SideActivation`] | one type at two paths; a dynamic child `dir` |
//! | `root.<org>` | [`OrgActivation`] | a static child under a dynamic one (`stats`), a dynamic under a dynamic (`repo`) |
//! | `root.left.<dir>` | [`DirActivation`] | a dynamic child of its own type (`sub`), recursive |
//! | `ping` / `pong` | [`mutual`] | mutual recursion: a static child one way, a dynamic child back |
//! | `loop` | [`selfish`] | children written `-> Self` and `-> Option<Self>` |
//! | `host` / `raw` | [`opaque`] | `#[child(opaque)]`: a child with no client (substrate's `SessionChild`), a hand-written `ChildRouter` |
//!
//! [`Fast`] is a transport that answers any call with `n` items at once, for
//! the per-event cost (`tests/bench.rs`).

use async_stream::stream;
use futures::Stream;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Every probe method's item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct N {
    pub n: u32,
}

impl plexus_client::FromClientError for N {
    fn from_client_error(_e: plexus_client::ClientError) -> Self {
        N { n: u32::MAX }
    }
}

fn ticks(n: u32) -> impl Stream<Item = N> + Send + 'static {
    stream! { for i in 0..n { yield N { n: i }; } }
}

/// The root.
#[derive(Debug, Clone, Default)]
pub struct RootActivation;

#[plexus_macros::activation(namespace = "root", description = "root", crate_path = "plexus_core")]
impl RootActivation {
    /// ticks
    #[plexus_macros::method(params(n = "count"))]
    pub async fn tick(&self, n: u32) -> impl Stream<Item = N> + Send + 'static {
        ticks(n)
    }
    /// left
    #[plexus_macros::child]
    pub fn left(&self) -> SideActivation {
        SideActivation
    }
    /// right: the same type as left, at another path
    #[plexus_macros::child]
    pub fn right(&self) -> SideActivation {
        SideActivation
    }
    /// an org by name
    #[plexus_macros::child]
    pub fn org(&self, name: &str) -> Option<OrgActivation> {
        Some(OrgActivation { name: name.to_string() })
    }
}

/// One type, two paths.
#[derive(Debug, Clone, Default)]
pub struct SideActivation;

#[plexus_macros::activation(namespace = "side", description = "side", crate_path = "plexus_core")]
impl SideActivation {
    /// ticks
    #[plexus_macros::method(params(n = "count"))]
    pub async fn tick(&self, n: u32) -> impl Stream<Item = N> + Send + 'static {
        ticks(n)
    }
    /// a dir by name (recursive)
    #[plexus_macros::child]
    pub fn dir(&self, name: &str) -> Option<DirActivation> {
        Some(DirActivation { name: name.to_string() })
    }
}

/// A dynamic child with a static and a dynamic child of its own.
#[derive(Debug, Clone, Default)]
pub struct OrgActivation {
    pub name: String,
}

#[plexus_macros::activation(namespace = "org", description = "org", crate_path = "plexus_core")]
impl OrgActivation {
    /// ticks
    #[plexus_macros::method(params(n = "count"))]
    pub async fn tick(&self, n: u32) -> impl Stream<Item = N> + Send + 'static {
        ticks(n)
    }
    /// stats
    #[plexus_macros::child]
    pub fn stats(&self) -> StatsActivation {
        StatsActivation
    }
    /// a repo by name
    #[plexus_macros::child]
    pub fn repo(&self, name: &str) -> Option<RepoActivation> {
        Some(RepoActivation { name: name.to_string() })
    }
}

/// A static child under a dynamic one.
#[derive(Debug, Clone, Default)]
pub struct StatsActivation;

#[plexus_macros::activation(namespace = "stats", description = "stats", crate_path = "plexus_core")]
impl StatsActivation {
    /// ticks
    #[plexus_macros::method(params(n = "count"))]
    pub async fn tick(&self, n: u32) -> impl Stream<Item = N> + Send + 'static {
        ticks(n)
    }

    /// Each number sent in, doubled — a method with an input half (BIDIR-6)
    /// on a child: it is lifted and named like any other.
    #[plexus_macros::method]
    pub async fn double(&self, ns: plexus_core::plexus::Input<N>) -> impl Stream<Item = N> + Send + 'static {
        use futures::StreamExt;
        ns.into_stream().map(|n| N { n: n.n * 2 })
    }
}

/// A dynamic child under a dynamic one.
#[derive(Debug, Clone, Default)]
pub struct RepoActivation {
    pub name: String,
}

#[plexus_macros::activation(namespace = "repo", description = "repo", crate_path = "plexus_core")]
impl RepoActivation {
    /// ticks
    #[plexus_macros::method(params(n = "count"))]
    pub async fn tick(&self, n: u32) -> impl Stream<Item = N> + Send + 'static {
        ticks(n)
    }
}

/// A dynamic child of its own type.
#[derive(Debug, Clone, Default)]
pub struct DirActivation {
    pub name: String,
}

#[plexus_macros::activation(namespace = "dir", description = "dir", crate_path = "plexus_core")]
impl DirActivation {
    /// ticks
    #[plexus_macros::method(params(n = "count"))]
    pub async fn tick(&self, n: u32) -> impl Stream<Item = N> + Send + 'static {
        ticks(n)
    }
    /// a sub dir by name
    #[plexus_macros::child]
    pub fn sub(&self, name: &str) -> Option<DirActivation> {
        Some(DirActivation { name: name.to_string() })
    }
}

/// Mutual recursion across two types: ping has the static child pong, pong
/// the dynamic child ping. Neither expansion can see the cycle.
pub mod mutual {
    use super::*;

    #[derive(Debug, Clone, Default)]
    pub struct PingActivation;

    #[plexus_macros::activation(namespace = "ping", description = "p", crate_path = "plexus_core")]
    impl PingActivation {
        /// ticks
        #[plexus_macros::method(params(n = "count"))]
        pub async fn tick(&self, n: u32) -> impl Stream<Item = N> + Send + 'static {
            ticks(n)
        }
        /// pong
        #[plexus_macros::child]
        pub fn pong(&self) -> PongActivation {
            PongActivation
        }
    }

    #[derive(Debug, Clone, Default)]
    pub struct PongActivation;

    #[plexus_macros::activation(namespace = "pong", description = "p", crate_path = "plexus_core")]
    impl PongActivation {
        /// ticks
        #[plexus_macros::method(params(n = "count"))]
        pub async fn tick(&self, n: u32) -> impl Stream<Item = N> + Send + 'static {
            ticks(n)
        }
        /// ping, by name
        #[plexus_macros::child]
        pub fn ping(&self, _name: &str) -> Option<PingActivation> {
            Some(PingActivation)
        }
    }
}

/// Children written with `Self`.
pub mod selfish {
    use super::*;

    #[derive(Debug, Clone, Default)]
    pub struct LoopActivation;

    #[plexus_macros::activation(namespace = "loop", description = "l", crate_path = "plexus_core")]
    impl LoopActivation {
        /// ticks
        #[plexus_macros::method(params(n = "count"))]
        pub async fn tick(&self, n: u32) -> impl Stream<Item = N> + Send + 'static {
            ticks(n)
        }
        /// itself, statically
        #[plexus_macros::child]
        pub fn me(&self) -> Self {
            LoopActivation
        }
        /// itself, by name
        #[plexus_macros::child]
        pub fn again(&self, _name: &str) -> Option<Self> {
            Some(LoopActivation)
        }
    }
}

/// `#[child(opaque)]`: children with no client — no accessor, no variant,
/// routed as before.
pub mod opaque;

/// A transport that answers any call with `self.0` data items `{"n": i}`, then done.
pub struct Fast(pub u32);

impl plexus_client::Transport for Fast {
    fn call(&self, _m: plexus_client::MethodRef, _p: serde_json::Value) -> plexus_client::ItemStream<'_> {
        let n = self.0;
        Box::pin(futures::stream::iter(
            (0..n)
                .map(|i| Ok(plexus_client::StreamItem::data(serde_json::json!({ "n": i }))))
                .chain(std::iter::once(Ok(plexus_client::StreamItem::done()))),
        ))
    }
}
