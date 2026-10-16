//! MACRO-20 — what `#[plexus_macros::activation]` emits for its static
//! children: `ChildRouter::forward_policy_for` (a static child is the same
//! service: `SameService`, unless `#[child(forward = "...")]` says
//! otherwise), `router_schema` / `router_static_child_schemas` (the
//! parent describes its children, grandchildren under their path), and
//! `Activation::static_child_schemas` (the same list, for a hub). Then a
//! call through the parent: a same-service child sees the caller's
//! metadata, an identity-only one does not.

use async_stream::stream;
use futures::stream::{Stream, StreamExt};
use plexus_core::plexus::{Activation, AuthContext, ChildRouter, PlexusStreamItem};
use serde_json::{json, Value};

#[derive(Clone)]
struct Probe;

#[plexus_macros::activation(namespace = "probe", version = "1.0.0", crate_path = "plexus_core")]
impl Probe {
    /// The caller's metadata, as the child sees it.
    #[plexus_macros::method]
    async fn seen(&self, auth: &AuthContext) -> impl Stream<Item = Value> + Send + 'static {
        let v = json!({ "user": auth.user_id, "metadata": auth.metadata });
        stream! { yield v; }
    }

    /// A grandchild.
    #[plexus_macros::child]
    fn deeper(&self) -> Leaf {
        Leaf
    }
}

#[derive(Clone)]
struct Leaf;

#[plexus_macros::activation(namespace = "leaf", version = "1.0.0", crate_path = "plexus_core")]
impl Leaf {
    /// Pong.
    #[plexus_macros::method]
    async fn ping(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "pong".into(); }
    }
}

#[derive(Clone)]
struct Parent;

#[plexus_macros::activation(namespace = "parent", version = "1.0.0", crate_path = "plexus_core")]
impl Parent {
    /// Pong.
    #[plexus_macros::method]
    async fn ping(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "pong".into(); }
    }

    /// The same service (the default).
    #[plexus_macros::child]
    fn kin(&self) -> Probe {
        Probe
    }

    /// A deputy.
    #[plexus_macros::child(forward = "identity_only")]
    fn deputy(&self) -> Probe {
        Probe
    }

    #[plexus_macros::child(forward = "pass_through")]
    fn through(&self) -> Leaf {
        Leaf
    }

    #[plexus_macros::child(forward = "anonymous")]
    fn nobody(&self) -> Probe {
        Probe
    }

    /// A dynamic child: the framework default unless it names one.
    #[plexus_macros::child]
    fn planet(&self, _name: &str) -> Option<Leaf> {
        Some(Leaf)
    }
}

fn policy(r: &dyn ChildRouter, ns: &str) -> Option<&'static str> {
    r.forward_policy_for(ns).map(|p| p.name().as_str())
}

#[test]
fn a_static_child_is_the_same_service_unless_it_says_otherwise() {
    assert_eq!(policy(&Parent, "kin"), Some("same_service"));
    assert_eq!(policy(&Parent, "deputy"), Some("identity_only"));
    assert_eq!(policy(&Parent, "through"), Some("pass_through"));
    assert_eq!(policy(&Parent, "nobody"), Some("anonymous"));
    // The dynamic child, and any name: no opinion (route_to_child's
    // IdentityOnly default).
    assert_eq!(policy(&Parent, "mars"), None);
    // An activation with no #[child] keeps the trait default.
    assert_eq!(policy(&Leaf, "anything"), None);
}

#[test]
fn the_parent_lists_its_static_children_and_theirs_by_path() {
    let listed: Vec<(String, String)> =
        Parent.router_static_child_schemas().into_iter().map(|(p, s)| (p, s.namespace)).collect();
    let want: Vec<(String, String)> = [
        ("kin", "probe"),
        ("kin.deeper", "leaf"),
        ("deputy", "probe"),
        ("deputy.deeper", "leaf"),
        ("through", "leaf"),
        ("nobody", "probe"),
        ("nobody.deeper", "leaf"),
    ]
    .into_iter()
    .map(|(a, b)| (a.to_string(), b.to_string()))
    .collect();
    assert_eq!(listed, want);
    // The same list through Activation, which a hub's registry holds.
    assert_eq!(Activation::static_child_schemas(&Parent).len(), want.len());
    // Each activation describes itself; a leaf lists no children.
    assert_eq!(Leaf.router_schema().unwrap().namespace, "leaf");
    assert!(Leaf.router_static_child_schemas().is_empty());
    assert!(Activation::static_child_schemas(&Leaf).is_empty());
}

async fn seen(child: &str) -> Value {
    let caller = AuthContext::new("u1".into(), "s1".into(), vec![], json!({"principal": "key:abc", "ceiling": ["viewer@A"]}));
    let stream = Activation::call(&Parent, &format!("{child}.seen"), json!({}), Some(&caller), None).await.unwrap();
    let items: Vec<Value> = stream
        .filter_map(|i| async move {
            match i {
                PlexusStreamItem::Data { content, .. } => Some(content),
                _ => None,
            }
        })
        .collect()
        .await;
    items.into_iter().next().unwrap()
}

#[tokio::test]
async fn a_same_service_child_sees_the_caller_whole() {
    let v = seen("kin").await;
    assert_eq!(v["user"], json!("u1"));
    assert_eq!(v["metadata"], json!({"principal": "key:abc", "ceiling": ["viewer@A"]}));
}

#[tokio::test]
async fn an_identity_only_child_does_not() {
    let v = seen("deputy").await;
    assert_eq!(v["user"], json!("u1"));
    assert_eq!(v["metadata"], Value::Null);
}

#[tokio::test]
async fn an_anonymous_child_sees_no_one() {
    let v = seen("nobody").await;
    assert_eq!(v["user"], json!("anonymous"));
    assert_eq!(v["metadata"], Value::Null);
}
