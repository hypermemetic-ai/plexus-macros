//! RETURNS-46 · the aggregate up the tree: how a call made through a
//! parent's client yields the parent's aggregate.
//!
//! An activation's aggregate (`{Ns}Event`) nests its children's: a static
//! child is a variant carrying the child's aggregate, a dynamic child a
//! variant carrying the instance's name and its aggregate. A generated client
//! `XClient<T, L = Root>` carries a [`Lift`] `L` from its own aggregate into
//! the aggregate of the client it was reached from:
//!
//! | client | `L` | its `.events()` yield |
//! |---|---|---|
//! | `TreeClient::new(t)` | [`Root`] | `TreeEvent` |
//! | `tree.branch()` | `Static<Root, BranchEvent, TreeEvent>` | `TreeEvent::Branch(..)` |
//! | `tree.item("a")` | `Named<Root, ItemEvent, TreeEvent>` | `TreeEvent::Item { name: "a", .. }` |
//! | `tree.branch().leaf()` | `Static<Static<Root, ..>, FoliageEvent, BranchEvent>` | `TreeEvent::Branch(BranchEvent::Leaf(..))` |
//!
//! The lift is a type: [`Root`] is nothing at all (an event of a client
//! reached through nothing is its own, at no cost); [`Static`] is the
//! parent's lift and a function pointer; [`Named`] adds the instance's name,
//! an `Arc<str>` each event shares. No closure, no allocation but the
//! variant's own box.
//!
//! [`Aggregated`] names a client's own aggregate — how a parent's expansion
//! names a child's aggregate without guessing it from the child's type name
//! (a child's aggregate is named by its namespace, its client by its stem).

use std::fmt;
use std::sync::Arc;

/// The aggregate a generated client's calls are of: its activation's
/// `{Ns}Event`. Implemented by every generated client, whatever its
/// transport and whatever it lifts into; a parent's aggregate names a
/// child's as `<InputsClient<()> as Aggregated>::Event`.
pub trait Aggregated {
    /// The activation's own aggregate.
    type Event;
}

/// An activation that has a generated client (RETURNS-46). The activation
/// macro implements it beside the server half of every activation it emits
/// a client for; a parent's expansion holds each `#[child]` to it, so a child
/// without a client is told what to do.
#[diagnostic::on_unimplemented(
    message = "`{Self}` has no client, so the parent's client cannot reach it as a `#[child]`",
    label = "a `#[child]` of an activation with a client must have a client too",
    note = "mark the child `#[plexus_macros::child(opaque)]`: no accessor, no variant in the parent's aggregate, routed as before — or give `{Self}` a client (name it `…Activation`, or add `client`)"
)]
pub trait HasClient {}

/// A lift from an aggregate `E` into an enclosing one, [`Lift::Out`]: how
/// many levels of variants to wrap an event in, and which instance names.
pub trait Lift<E>: Clone + Send + Sync + 'static {
    /// The aggregate an event ends in.
    type Out;
    /// Lift one event.
    fn lift(&self, event: E) -> Self::Out;
}

/// No lift: an event is its own aggregate's. Zero-sized.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Root;

impl<E> Lift<E> for Root {
    type Out = E;
    #[inline(always)]
    fn lift(&self, event: E) -> E {
        event
    }
}

/// A static child's lift: wrap the child's event `C` in the parent's
/// variant (`P`), then lift as the parent does (`L`).
pub struct Static<L, C, P> {
    outer: L,
    wrap: fn(C) -> P,
}

impl<L, C, P> Static<L, C, P> {
    /// `wrap`, then `outer`.
    pub fn new(outer: L, wrap: fn(C) -> P) -> Self {
        Self { outer, wrap }
    }
}

impl<L: Lift<P>, C: 'static, P: 'static> Lift<C> for Static<L, C, P> {
    type Out = L::Out;
    #[inline]
    fn lift(&self, event: C) -> L::Out {
        self.outer.lift((self.wrap)(event))
    }
}

/// A dynamic child's lift: the instance's name and the child's event `C` in
/// the parent's variant (`P`), then lifted as the parent does (`L`).
pub struct Named<L, C, P> {
    outer: L,
    name: Arc<str>,
    wrap: fn(Arc<str>, C) -> P,
}

impl<L, C, P> Named<L, C, P> {
    /// The instance `name`: `wrap`, then `outer`.
    pub fn new(outer: L, name: Arc<str>, wrap: fn(Arc<str>, C) -> P) -> Self {
        Self { outer, name, wrap }
    }

    /// The instance's name.
    pub fn name(&self) -> &Arc<str> {
        &self.name
    }
}

impl<L: Lift<P>, C: 'static, P: 'static> Lift<C> for Named<L, C, P> {
    type Out = L::Out;
    #[inline]
    fn lift(&self, event: C) -> L::Out {
        self.outer.lift((self.wrap)(Arc::clone(&self.name), event))
    }
}

impl<L: Clone, C, P> Clone for Static<L, C, P> {
    fn clone(&self) -> Self {
        Self { outer: self.outer.clone(), wrap: self.wrap }
    }
}

impl<L: Clone, C, P> Clone for Named<L, C, P> {
    fn clone(&self) -> Self {
        Self { outer: self.outer.clone(), name: Arc::clone(&self.name), wrap: self.wrap }
    }
}

impl<L: fmt::Debug, C, P> fmt::Debug for Static<L, C, P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Static").field("into", &std::any::type_name::<P>()).field("outer", &self.outer).finish()
    }
}

impl<L: fmt::Debug, C, P> fmt::Debug for Named<L, C, P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Named")
            .field("name", &self.name)
            .field("into", &std::any::type_name::<P>())
            .field("outer", &self.outer)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq)]
    enum Leaf {
        Fall(u32),
    }
    #[derive(Debug, PartialEq)]
    enum Branch {
        Leaf(Box<Leaf>),
    }
    #[derive(Debug, PartialEq)]
    enum Tree {
        Branch(Box<Branch>),
        Item { name: Arc<str>, event: Box<Leaf> },
    }

    #[test]
    fn lifts_compose_one_level_at_a_time() {
        let branch: Static<Root, Branch, Tree> = Static::new(Root, |b| Tree::Branch(Box::new(b)));
        let leaf = Static::new(branch, |l| Branch::Leaf(Box::new(l)));
        assert_eq!(leaf.lift(Leaf::Fall(1)), Tree::Branch(Box::new(Branch::Leaf(Box::new(Leaf::Fall(1))))));
        let item: Named<Root, Leaf, Tree> = Named::new(Root, Arc::from("a"), |name, l| Tree::Item { name, event: Box::new(l) });
        assert_eq!(item.lift(Leaf::Fall(2)), Tree::Item { name: Arc::from("a"), event: Box::new(Leaf::Fall(2)) });
    }

    #[test]
    fn the_root_lift_is_nothing() {
        assert_eq!(std::mem::size_of::<Root>(), 0);
        fn send_sync<T: Send + Sync + Clone>() {}
        send_sync::<Static<Root, Leaf, Tree>>();
        send_sync::<Named<Root, Leaf, Tree>>();
    }
}
