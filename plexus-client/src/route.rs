//! RETURNS-46 · where a generated client's calls go, and whether they may.
//!
//! A [`Route`] is a client's path (a [`RoutePath`]) and, for a client below a
//! dynamic instance whose name the service would route elsewhere, why
//! nothing may be sent. A child's route is its parent's, joined; a refusal
//! is inherited, so `root.org("acme.stats").stats().tick(..)` fails as
//! `root.org("acme.stats").tick(..)` does — at once, with
//! [`ClientError::InvalidName`], sending nothing.

use std::sync::Arc;

use crate::error::{ClientError, NameRefusal};
use crate::transport::{MethodRef, RoutePath};

/// A generated client's route: its path, and a refusal if an instance name
/// on the way to it cannot be routed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Route {
    path: RoutePath,
    refused: Option<Arc<(String, NameRefusal)>>,
}

impl Route {
    /// The activation at its own namespace.
    pub const ROOT: Route = Route { path: RoutePath::EMPTY, refused: None };

    /// The activation mounted at `path` (its namespace, or empty, is
    /// [`Route::ROOT`]).
    pub fn at(path: impl Into<RoutePath>, namespace: &str) -> Route {
        let path = path.into();
        if path.as_str() == namespace {
            Route::ROOT
        } else {
            Route { path, refused: None }
        }
    }

    /// The dotted path: `namespace` when mounted at its own.
    pub fn path<'a>(&'a self, namespace: &'a str) -> &'a str {
        if self.path.is_empty() {
            namespace
        } else {
            self.path.as_str()
        }
    }

    /// The route of the static child `segment`. `joined` is
    /// `"{namespace}.{segment}"`, known when the program is written: a
    /// child of a client at its own namespace costs no allocation.
    pub fn child(&self, segment: &str, joined: &'static str) -> Route {
        let path = if self.path.is_empty() {
            RoutePath::from_static(joined)
        } else {
            self.path.join(segment)
        };
        Route { path, refused: self.refused.clone() }
    }

    /// The route of the dynamic instance `name`, refused when the service
    /// would route `name` elsewhere: empty, dotted, one of the parent's
    /// static children's `statics`, or one of its `methods`.
    pub fn instance(&self, namespace: &str, name: &str, statics: &[&str], methods: &[&str]) -> Route {
        let path = if self.path.is_empty() {
            RoutePath::from(format!("{namespace}.{name}"))
        } else {
            self.path.join(name)
        };
        let refusal = if name.is_empty() {
            Some(NameRefusal::Empty)
        } else if name.contains('.') {
            Some(NameRefusal::Dotted)
        } else if statics.contains(&name) {
            Some(NameRefusal::StaticChild)
        } else if methods.contains(&name) {
            Some(NameRefusal::Method)
        } else {
            None
        };
        let refused = match (&self.refused, refusal) {
            (Some(earlier), _) => Some(earlier.clone()),
            (None, Some(why)) => Some(Arc::new((name.to_owned(), why))),
            (None, None) => None,
        };
        Route { path, refused }
    }

    /// `method` sent along this route, or why it may not be.
    pub fn method(&self, method: MethodRef) -> Result<MethodRef, ClientError> {
        match &self.refused {
            None => Ok(if self.path.is_empty() { method } else { method.at(self.path.clone()) }),
            Some(r) => Err(ClientError::InvalidName { name: r.0.clone(), reason: r.1 }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_join_and_refuse() {
        let root = Route::ROOT;
        assert_eq!(root.path("tree"), "tree");
        let branch = root.child("branch", "tree.branch");
        assert_eq!(branch.path("branch"), "tree.branch");
        let a = root.instance("tree", "a", &["branch"], &["count"]);
        assert_eq!(a.path("item"), "tree.a");
        assert_eq!(a.method(MethodRef::new("item", "watch")).unwrap().qualified(), "tree.a.watch");
        for (name, why) in [("", NameRefusal::Empty), ("a.b", NameRefusal::Dotted), ("branch", NameRefusal::StaticChild), ("count", NameRefusal::Method)] {
            let r = root.instance("tree", name, &["branch"], &["count"]);
            assert_eq!(r.method(MethodRef::new("item", "watch")), Err(ClientError::InvalidName { name: name.into(), reason: why }));
            // inherited below
            let below = r.child("leaf", "item.leaf").instance("item", "ok", &[], &[]);
            assert!(below.method(MethodRef::new("x", "y")).is_err());
        }
    }
}
