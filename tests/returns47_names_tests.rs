//! RETURNS-47 · no `…Activation` suffix, a trait only when named,
//! `impl Trait for Type`, and a client that implements no trait.
//!
//! - `#[activation] impl Tickets` is a whole declaration: namespace
//!   `tickets`, client `TicketsClient`, and no trait.
//! - `impl TicketsActivation` works as before (the stem), and emits no trait
//!   either; `trait = "Tickets2"` emits one.
//! - `#[activation] impl forge::Forge for GitHub` serves the trait's methods
//!   under `forge`; the type names nothing.
//! - The client's one-item methods answer `Result<T, ClientError>` whatever
//!   `T` is — `u64`, `bool`, `Vec<String>` — with no `FromClientError`.

use futures::{Stream, TryStreamExt};
use plexus_client::ClientError;
use plexus_core::plexus::{Activation, InProcessTransport};

/// `impl Tickets`: no suffix, nothing in the attribute.
mod bare {
    use futures::Stream;

    #[derive(Clone)]
    pub struct Tickets;

    /// Tickets, with no suffix.
    #[plexus_macros::activation]
    impl Tickets {
        /// Every ticket's id.
        pub async fn ids(&self, to: u32) -> impl Stream<Item = u32> + Send + 'static {
            futures::stream::iter(1..=to)
        }

        /// How many there are.
        #[plexus_macros::method]
        pub async fn total(&self) -> u64 {
            3
        }

        /// Whether the store is up.
        #[plexus_macros::method]
        pub async fn up(&self) -> bool {
            true
        }

        /// The names, filtered.
        #[plexus_macros::method]
        pub async fn names(&self, prefix: String) -> Vec<String> {
            ["MNG-1", "MNG-2", "OTHER-1"].iter().filter(|n| n.starts_with(&prefix)).map(|n| n.to_string()).collect()
        }
    }
}

/// `impl TicketsActivation`: the stem, as before — and a trait named
/// `Tickets` of the author's own beside it, which the macro no longer
/// emits (it would be defined twice).
mod suffixed {
    use futures::Stream;

    /// The author's own `Tickets`: no trait of that name is emitted.
    #[allow(dead_code)]
    pub trait Tickets {}

    #[derive(Clone)]
    pub struct TicketsActivation;

    /// Tickets, the old name.
    #[plexus_macros::activation]
    impl TicketsActivation {
        /// Every ticket's id.
        pub async fn ids(&self, to: u32) -> impl Stream<Item = u32> + Send + 'static {
            futures::stream::iter(1..=to)
        }
    }
}

/// `trait = "Tickets2"`: a trait, because it is named.
mod named {
    use futures::Stream;

    #[derive(Clone)]
    pub struct Tickets;

    /// Tickets, with a named trait.
    #[plexus_macros::activation(trait = "Tickets2")]
    impl Tickets {
        /// Every ticket's id.
        pub async fn ids(&self, to: u32) -> impl Stream<Item = u32> + Send + 'static {
            futures::stream::iter(1..=to)
        }

        /// How many there are.
        #[plexus_macros::method]
        pub async fn total(&self) -> u64 {
            3
        }
    }
}

/// The contract, written by hand: GRAFT-1's `forge`.
mod forge {
    use futures::Stream;

    /// A forge: where repositories live.
    #[allow(async_fn_in_trait)]
    pub trait Forge {
        async fn total(&self) -> u64;
        async fn up(&self) -> bool;
        async fn names(&self, prefix: String) -> Vec<String>;
        async fn events(&self, n: u32) -> impl Stream<Item = u32> + Send + 'static;
    }
}

#[derive(Clone)]
pub struct GitHub {
    repos: Vec<&'static str>,
}

/// GitHub, as a forge.
#[plexus_macros::activation]
impl forge::Forge for GitHub {
    /// How many repositories.
    async fn total(&self) -> u64 {
        self.repos.len() as u64
    }

    /// Whether GitHub answers.
    async fn up(&self) -> bool {
        true
    }

    /// The repositories' names, filtered.
    async fn names(&self, prefix: String) -> Vec<String> {
        self.repos.iter().filter(|n| n.starts_with(&prefix)).map(|n| n.to_string()).collect()
    }

    /// 0..n.
    async fn events(&self, n: u32) -> impl Stream<Item = u32> + Send + 'static {
        futures::stream::iter(0..n)
    }
}

fn github() -> GitHub {
    GitHub { repos: vec!["menger", "plexus-core", "synapse"] }
}

#[test]
fn impl_tickets_with_no_suffix_serves_tickets() {
    let schema = bare::Tickets.plugin_schema();
    assert_eq!(schema.namespace, "tickets");
    let mut names: Vec<&str> = schema.methods.iter().map(|m| m.name.as_str()).collect();
    names.sort();
    assert_eq!(names, ["ids", "names", "total", "up"]);
    assert_eq!(schema.description, "Tickets, with no suffix.");
}

#[tokio::test]
async fn impl_tickets_generates_tickets_client_whose_one_item_methods_answer_result() {
    let client = bare::TicketsClient::new(InProcessTransport::new(bare::Tickets));
    let total: Result<u64, ClientError> = client.total().await;
    assert_eq!(total.unwrap(), 3);
    let up: Result<bool, ClientError> = client.up().await;
    assert!(up.unwrap());
    let names: Result<Vec<String>, ClientError> = client.names("MNG".into()).await;
    assert_eq!(names.unwrap(), ["MNG-1", "MNG-2"]);
    let ids: Vec<u32> = client.ids(3).try_collect().await.unwrap();
    assert_eq!(ids, [1, 2, 3]);
}

#[tokio::test]
async fn an_activation_named_with_the_suffix_still_takes_its_stem_and_emits_no_trait() {
    assert_eq!(suffixed::TicketsActivation.plugin_schema().namespace, "tickets");
    let client = suffixed::TicketsClient::new(InProcessTransport::new(suffixed::TicketsActivation));
    let ids: Vec<u32> = client.ids(2).try_collect().await.unwrap();
    assert_eq!(ids, [1, 2]);
    // The author's `trait Tickets` is the only one: the macro emitted none.
    impl suffixed::Tickets for () {}
}

fn is_tickets2<T: named::Tickets2>() {}

#[tokio::test]
async fn a_named_trait_is_emitted_and_implemented_by_the_activation() {
    is_tickets2::<named::Tickets>();
    // The trait's methods are the activation's, boxed.
    let total = <named::Tickets as named::Tickets2>::total(&named::Tickets).await;
    assert_eq!(total, 3);
    let ids: Vec<u32> = futures::StreamExt::collect(<named::Tickets as named::Tickets2>::ids(&named::Tickets, 2)).await;
    assert_eq!(ids, [1, 2]);
    // The client answers `Result`s, and implements no trait (a trybuild case).
    let client = named::Tickets2Client::new(InProcessTransport::new(named::Tickets));
    let total: Result<u64, ClientError> = client.total().await;
    assert_eq!(total.unwrap(), 3);
}

#[test]
fn impl_forge_for_github_serves_the_traits_methods_under_forge() {
    let schema = github().plugin_schema();
    assert_eq!(schema.namespace, "forge");
    let mut names: Vec<&str> = schema.methods.iter().map(|m| m.name.as_str()).collect();
    names.sort();
    assert_eq!(names, ["events", "names", "total", "up"]);
    assert_eq!(schema.description, "GitHub, as a forge.");
    let total = schema.methods.iter().find(|m| m.name == "total").unwrap();
    assert_eq!(total.description, "How many repositories.");
}

#[tokio::test]
async fn the_forge_client_answers_result_of_u64_for_a_u64_method() {
    let client = ForgeClient::new(InProcessTransport::new(github()));
    let total: Result<u64, ClientError> = client.total().await;
    assert_eq!(total.unwrap(), 3);
    let up: Result<bool, ClientError> = client.up().await;
    assert!(up.unwrap());
    let names: Result<Vec<String>, ClientError> = client.names("p".into()).await;
    assert_eq!(names.unwrap(), ["plexus-core"]);
    let events: Vec<u32> = client.events(3).try_collect().await.unwrap();
    assert_eq!(events, [0, 1, 2]);
}

#[tokio::test]
async fn github_is_still_a_forge_called_through_the_trait() {
    use forge::Forge;
    assert_eq!(github().total().await, 3);
    let s = github().events(2).await;
    let got: Vec<u32> = futures::StreamExt::collect(s).await;
    assert_eq!(got, [0, 1]);
    fn is_stream<S: Stream<Item = u32>>(_: &S) {}
    is_stream(&github().events(0).await);
}

#[tokio::test]
async fn a_client_failure_is_an_err_not_a_value_folded_into_the_item() {
    // Nothing scripted: the memory transport refuses the call.
    let client = bare::TicketsClient::new(std::sync::Arc::new(plexus_client::MemoryTransport::new()));
    let total: Result<u64, ClientError> = client.total().await;
    assert!(total.is_err(), "{total:?}");
}
