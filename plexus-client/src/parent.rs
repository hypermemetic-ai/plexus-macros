//! NEST-16 · what a graft's macro reads of its parent's client.
//!
//! `#[plexus_macros::activation(parent = ShopClient)]` infers what each
//! method calls of its parent from the call chains rooted at the injected
//! `parent` (`parent.items()`, `parent.ui().slots().list()`). Which link of a
//! chain is the parent's method — and which are child accessors before it,
//! or a stream's own methods after it (`.collect()`) — is known only to the
//! parent's client: every generated client answers it through
//! [`ParentShape`], and, where its server half is built, the method's shape
//! hash through [`ParentHash`].

use core::marker::PhantomData;

/// A client's route grammar, for inferring a graft's needs.
pub trait ParentShape {
    /// The activation namespace the client speaks to.
    fn namespace() -> &'static str;

    /// `segments` are a call chain's names as the Rust code wrote them
    /// (`["ui", "slots", "list", "collect"]`). The wire path of the method the
    /// chain calls, below this client's node (`"ui.slots.list"`), and its
    /// shape hash where known; `None` when the chain names no method of this
    /// client (a dynamic child's or a peer slot's accessor, which take a name
    /// only known when the code runs).
    fn resolve(segments: &[&str]) -> Option<(String, Option<String>)>;
}

/// A client's methods' shape hashes (NEST-7's `Interface::method_hash`),
/// emitted beside its activation's server half.
pub trait ParentHash {
    /// The shape hash of the method named `method` on the wire.
    fn method_hash(method: &str) -> Option<String>;
}

#[doc(hidden)]
pub mod __probe {
    //! Autoref: a client's [`ParentHash`](super::ParentHash) when it has
    //! one, `None` when it does not (a client-only build).
    use super::*;

    #[derive(Debug)]
    pub struct Probe<C>(PhantomData<C>);

    impl<C> Probe<C> {
        #[allow(clippy::new_without_default)]
        pub fn new() -> Self {
            Probe(PhantomData)
        }
    }

    pub trait ViaHash {
        fn __plexus_hash(&self, method: &str) -> Option<String>;
    }

    impl<C: super::ParentHash> ViaHash for &Probe<C> {
        fn __plexus_hash(&self, method: &str) -> Option<String> {
            C::method_hash(method)
        }
    }

    pub trait ViaNone {
        fn __plexus_hash(&self, method: &str) -> Option<String>;
    }

    impl<C> ViaNone for Probe<C> {
        fn __plexus_hash(&self, _method: &str) -> Option<String> {
            None
        }
    }
}
