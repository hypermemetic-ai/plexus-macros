//! RETURNS-7: the conformance fixture. One activation with one method per
//! signature form (RETURNS-1 decisions 2, 3, 4, 10).
//!
//! Every generator is tested against this one activation: the macro's server
//! half (RETURNS-8) and client half (RETURNS-9), the Haskell protocol
//! (RETURNS-12), synapse's IR (RETURNS-13), and hub-codegen's TS and Rust
//! clients (RETURNS-17, RETURNS-18). `expected/` holds what it must produce.
//! The calls and their params are in `expected/calls.json`; `README.md`
//! says who round-trips what.
//!
//! | method | signature form | cardinality |
//! |---|---|---|
//! | `ping()` | `-> Pong` | one, infallible |
//! | `lookup(id)` | `-> Result<Thing, LookupError>` | one, fallible |
//! | `count(to)` | `-> impl Stream<Item = Tick>` | many |
//! | `scan(prefix)` | `-> impl Stream<Item = Result<Row, ScanError>>` | many, fallible; an err does not end it |
//! | `load(n)` | `-> impl Stream<Item = Result<Progress<Loaded, LoadStep>, LoadError>>` | many, typed progress |
//! | `warm()` | `-> impl Stream<Item = Progress<Warmed>>` | many, default `Note` progress |
//!
//! Every body is deterministic: same params, same items, same order.
//!
//! The namespace is implicit, `conformance` from the struct name (RETURNS-1
//! decision 7). The crate joins the plexus-macros workspace in RETURNS-8,
//! when the macro learns these signature forms. Until then nothing compiles
//! this file, but `types.rs` already compiles: plexus-core's
//! `tests/conformance_fixture.rs` includes it by sibling path.
//!
//! A change here or under `expected/` is a contract change. It names the
//! RETURNS-1 decision it follows.

pub mod types;
pub use types::*;

use async_stream::stream;
use futures::Stream;
use plexus_client::{Note, Progress};

/// The one row of `scan`'s table it cannot read.
const UNREADABLE: &str = "a2";

/// `scan`'s whole table, in scan order.
const ROWS: [&str; 4] = ["a1", "a2", "a3", "b1"];

/// The most `load` takes; more is a `LoadError::TooMany`.
const MAX_LOAD: u32 = 3;

/// The activation. Stateless.
#[derive(Debug, Clone, Copy, Default)]
pub struct Conformance;

#[plexus_macros::activation(
    description = "Conformance fixture: one method per signature form",
    client,
    crate_path = "plexus_core"
)]
impl Conformance {
    // -> T: one item, infallible.
    /// Always answers pong.
    #[plexus_macros::method]
    pub async fn ping(&self) -> Pong {
        Pong {
            message: "pong".into(),
        }
    }

    // -> Result<T, E>: one item, fallible.
    /// The thing with this id; the id "missing" is not found.
    #[plexus_macros::method(params(id = "Thing id"))]
    pub async fn lookup(&self, id: String) -> Result<Thing, LookupError> {
        if id == "missing" {
            return Err(LookupError::NotFound { id });
        }
        Ok(Thing {
            name: format!("thing {id}"),
            id,
        })
    }

    // -> impl Stream<Item = T>: many items.
    /// Ticks from 1 up to `to`.
    #[plexus_macros::method(params(to = "Last tick"))]
    pub async fn count(&self, to: u32) -> impl Stream<Item = Tick> + Send + 'static {
        stream! {
            for n in 1..=to {
                yield Tick { n };
            }
        }
    }

    // -> impl Stream<Item = Result<T, E>>: many items, each can fail. The
    // unreadable row is an err item, and the scan carries on past it.
    /// The rows whose key starts with `prefix`, in key order.
    #[plexus_macros::method(params(prefix = "Key prefix"))]
    pub async fn scan(
        &self,
        prefix: String,
    ) -> impl Stream<Item = Result<Row, ScanError>> + Send + 'static {
        stream! {
            for key in ROWS.iter().filter(|k| k.starts_with(prefix.as_str())) {
                if *key == UNREADABLE {
                    yield Err(ScanError::Unreadable { key: key.to_string() });
                } else {
                    yield Ok(Row { key: key.to_string() });
                }
            }
        }
    }

    // -> impl Stream<Item = Result<Progress<T, P>, E>>: typed progress. One
    // `LoadStep` per batch, then the item. More than MAX_LOAD is refused: an
    // err that ends the stream because the method ends it.
    /// Loads `n` batches, reporting each, then says how many items it loaded.
    #[plexus_macros::method(params(n = "Batches to load, at most 3"))]
    pub async fn load(
        &self,
        n: u32,
    ) -> impl Stream<Item = Result<Progress<Loaded, LoadStep>, LoadError>> + Send + 'static {
        stream! {
            if n > MAX_LOAD {
                yield Err(LoadError::TooMany { n, max: MAX_LOAD });
                return;
            }
            for done in 1..=n {
                yield Ok(Progress::Update(LoadStep::Batch { done, of: n }));
            }
            yield Ok(Progress::Item(Loaded { items: n }));
        }
    }

    // -> impl Stream<Item = Progress<T>>: default progress, `P = Note`. One
    // note without a percentage, one with.
    /// Warms the cache, noting its progress, then says how many entries it holds.
    #[plexus_macros::method]
    pub async fn warm(&self) -> impl Stream<Item = Progress<Warmed>> + Send + 'static {
        stream! {
            yield Progress::Update(Note::new("cache cold"));
            yield Progress::Update(Note::new("cache warming").with_percentage(50.0));
            yield Progress::Item(Warmed { entries: 2 });
        }
    }
}
