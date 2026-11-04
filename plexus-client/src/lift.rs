//! RETURNS-46 · the aggregate up the tree: how a call made through a
//! parent's client yields the parent's aggregate.
//!
//! An activation's aggregate (`{Ns}Event`) nests its children's: a static
//! child is a variant carrying the child's aggregate, a dynamic child a
//! variant carrying the instance's name and its aggregate. A generated
//! client carries a [`Lift`] from its own aggregate into the aggregate of
//! the client it was reached from — the identity for a client made with
//! `new` or `at`, and one wrapping per level for a client a parent's
//! accessor made:
//!
//! ```ignore
//! let tree = TreeClient::new(&ws);                     // Lift<TreeEvent, TreeEvent>: identity
//! let leaf = tree.branch().leaf();                     // LeafClient<_, TreeEvent>
//! leaf.fall(2).events()                                // Events<'_, TreeEvent>:
//!     // TreeEvent::Branch(BranchEvent::Leaf(FoliageEvent::Fall(..)))
//! tree.item("a").watch(1).events()                     // TreeEvent::Item { name: "a", event: ItemEvent::Watch(..) }
//! ```
//!
//! So `select_all` of calls anywhere below one client is one stream of that
//! client's aggregate, matched with no wildcard; and a dynamic instance's
//! name is in each of its events because its accessor was given it.
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

/// A function from an aggregate `E` into an enclosing aggregate `A`: how
/// many levels of variants to wrap an event in, and which dynamic names.
///
/// Cheap to clone (a function pointer, or an `Arc`), `Send` and `Sync`.
pub struct Lift<E, A> {
    f: Inner<E, A>,
}

enum Inner<E, A> {
    Fn(fn(E) -> A),
    Closure(Arc<dyn Fn(E) -> A + Send + Sync>),
}

fn identity<E>(e: E) -> E {
    e
}

impl<E> Lift<E, E> {
    /// No lift: an event is its own aggregate's.
    pub fn identity() -> Self {
        Self { f: Inner::Fn(identity::<E>) }
    }
}

impl<E, A> Lift<E, A> {
    /// A lift that is a function pointer — a variant's constructor, say.
    pub fn from_fn(f: fn(E) -> A) -> Self {
        Self { f: Inner::Fn(f) }
    }

    /// A lift that is a closure — one that carries a dynamic child's name.
    pub fn new(f: impl Fn(E) -> A + Send + Sync + 'static) -> Self {
        Self { f: Inner::Closure(Arc::new(f)) }
    }

    /// Lift one event.
    pub fn apply(&self, event: E) -> A {
        match &self.f {
            Inner::Fn(f) => f(event),
            Inner::Closure(f) => f(event),
        }
    }

    /// `inner`, then this lift: a child's event wrapped by `inner` into
    /// this level's aggregate, then lifted as this level's are. What a
    /// parent's accessor gives the child's client.
    pub fn after<D>(&self, inner: impl Fn(D) -> E + Send + Sync + 'static) -> Lift<D, A>
    where
        E: 'static,
        A: 'static,
    {
        let outer = self.clone();
        Lift::new(move |d| outer.apply(inner(d)))
    }

    /// This lift, then `outer`.
    pub fn then<B>(self, outer: Lift<A, B>) -> Lift<E, B>
    where
        E: 'static,
        A: 'static,
        B: 'static,
    {
        Lift::new(move |e| outer.apply(self.apply(e)))
    }
}

impl<E, A> Clone for Lift<E, A> {
    fn clone(&self) -> Self {
        Self {
            f: match &self.f {
                Inner::Fn(f) => Inner::Fn(*f),
                Inner::Closure(f) => Inner::Closure(Arc::clone(f)),
            },
        }
    }
}

impl<E, A> fmt::Debug for Lift<E, A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Lift")
            .field("from", &std::any::type_name::<E>())
            .field("into", &std::any::type_name::<A>())
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
        Leaf(Leaf),
    }
    #[derive(Debug, PartialEq)]
    enum Tree {
        Branch(Branch),
        Item { name: String, event: Leaf },
    }

    #[test]
    fn lifts_compose_one_level_at_a_time() {
        let tree: Lift<Tree, Tree> = Lift::identity();
        let branch: Lift<Branch, Tree> = tree.after(Tree::Branch);
        let leaf: Lift<Leaf, Tree> = branch.after(Branch::Leaf);
        assert_eq!(leaf.apply(Leaf::Fall(1)), Tree::Branch(Branch::Leaf(Leaf::Fall(1))));
        let name = String::from("a");
        let item: Lift<Leaf, Tree> = tree.after(move |event| Tree::Item { name: name.clone(), event });
        assert_eq!(item.apply(Leaf::Fall(2)), Tree::Item { name: "a".into(), event: Leaf::Fall(2) });
        let then = Lift::from_fn(Leaf::Fall).then(leaf.clone());
        assert_eq!(then.apply(3), Tree::Branch(Branch::Leaf(Leaf::Fall(3))));
    }

    #[test]
    fn a_lift_is_send_sync_and_clone() {
        fn send_sync<T: Send + Sync + Clone>() {}
        send_sync::<Lift<String, u8>>();
    }
}
