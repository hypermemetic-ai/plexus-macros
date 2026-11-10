//! KEEL-27 · `#[activation(served = written)]`: what an unmarked fn of the
//! block is, read from how it is written.
//!
//! | the fn, unmarked | is |
//! | -- | -- |
//! | not `pub` | a helper, not served |
//! | `pub fn x(&self) -> A` | a static mount |
//! | `pub fn x(&self, name: &str) -> Option<A>` | a slot |
//! | `pub async fn x(&self, name: &str) -> Result<Box<dyn I>, PeerError>` | a fallible mount |
//! | any other `pub async fn` on `&self` | a method |
//! | `pub`, not on `&self` (`new`) | a helper |
//! | a `pub fn` that cannot be a mount | a compile error (trybuild, below) |
//! | a fn with a marker | what the marker says |
//!
//! The shelf is written twice — `marked` with every marker, `written` with
//! none — and the two are compared by what they serve: the service's IR,
//! the schema and its grants, the client's calls and what routing answers.

use async_stream::stream;
use futures::stream::{Stream, StreamExt};
use plexus_client::MemoryTransport;
use plexus_core::plexus::peer::{Peer, PeerError};
use plexus_core::plexus::{Activation, ChildRouter, HubTransport};
use plexus_core::DynamicHub;
use serde_json::{json, Value};
use std::sync::Arc;

// ------------------------------------------------------------------ the mounts, shared

/// The shelf's index.
#[derive(Clone)]
pub struct IndexActivation;

#[plexus_macros::activation]
impl IndexActivation {
    /// Every entry.
    pub async fn entries(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "dune".to_string(); yield "emma".to_string(); }
    }
}

/// One book.
#[derive(Clone)]
pub struct BookActivation {
    title: String,
}

#[plexus_macros::activation]
impl BookActivation {
    /// The book's title.
    pub async fn title(&self) -> impl Stream<Item = String> + Send + 'static {
        let title = self.title.clone();
        stream! { yield title; }
    }
}

/// Says hello.
#[derive(Clone)]
pub struct GreeterActivation;

#[plexus_macros::activation]
impl GreeterActivation {
    /// Hello.
    pub async fn hello(&self, name: String) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield format!("hello, {name}"); }
    }
}

// ------------------------------------------------------------------ the shelf, with every marker

pub mod marked {
    use super::*;

    /// A shelf of books.
    #[derive(Clone)]
    pub struct ShelfActivation;

    #[plexus_macros::activation(roles(viewer = ["titles", "count", "{book}.title"], editor = [viewer, "rename"]))]
    impl ShelfActivation {
        /// A constructor: not on `&self`, never served.
        pub fn new() -> Self {
            ShelfActivation
        }

        /// Every title.
        #[plexus_macros::method]
        pub async fn titles(&self) -> impl Stream<Item = String> + Send + 'static {
            let n = self.size();
            stream! { yield format!("{n} titles"); }
        }

        /// How many books.
        #[plexus_macros::method]
        pub async fn count(&self) -> u32 {
            self.size()
        }

        /// Rename a book.
        #[plexus_macros::method]
        pub async fn rename(&self, from: String, to: String) -> Result<String, String> {
            if from == "dune" { Ok(format!("{from} → {to}")) } else { Err(format!("no {from}")) }
        }

        /// The index.
        #[plexus_macros::child]
        pub fn index(&self) -> IndexActivation {
            IndexActivation
        }

        /// A book by its name.
        #[plexus_macros::child]
        pub fn book(&self, name: &str) -> Option<BookActivation> {
            (name == "dune").then(|| BookActivation { title: "Dune".into() })
        }

        /// A greeter, which may be away.
        #[plexus_macros::peer]
        pub async fn greeter(&self, name: &str) -> Result<Box<dyn Peer>, PeerError> {
            match name {
                "here" => Ok(Box::new(GreeterActivation)),
                _ => Err(PeerError::NotFound { name: name.into() }),
            }
        }

        /// A helper.
        fn size(&self) -> u32 {
            2
        }

        /// An async helper.
        #[allow(dead_code)]
        async fn audit(&self) -> impl Stream<Item = String> + Send + 'static {
            stream! { yield "audited".to_string(); }
        }
    }
}

// ------------------------------------------------------------------ the shelf, with none

pub mod written {
    use super::*;

    /// A shelf of books.
    #[derive(Clone)]
    pub struct ShelfActivation;

    #[plexus_macros::activation(served = written, roles(viewer = ["titles", "count", "{book}.title"], editor = [viewer, "rename"]))]
    impl ShelfActivation {
        /// A constructor: not on `&self`, never served.
        pub fn new() -> Self {
            ShelfActivation
        }

        /// Every title.
        pub async fn titles(&self) -> impl Stream<Item = String> + Send + 'static {
            let n = self.size();
            stream! { yield format!("{n} titles"); }
        }

        /// How many books.
        pub async fn count(&self) -> u32 {
            self.size()
        }

        /// Rename a book.
        pub async fn rename(&self, from: String, to: String) -> Result<String, String> {
            if from == "dune" { Ok(format!("{from} → {to}")) } else { Err(format!("no {from}")) }
        }

        /// The index.
        pub fn index(&self) -> IndexActivation {
            IndexActivation
        }

        /// A book by its name.
        pub fn book(&self, name: &str) -> Option<BookActivation> {
            (name == "dune").then(|| BookActivation { title: "Dune".into() })
        }

        /// A greeter, which may be away.
        pub async fn greeter(&self, name: &str) -> Result<Box<dyn Peer>, PeerError> {
            match name {
                "here" => Ok(Box::new(GreeterActivation)),
                _ => Err(PeerError::NotFound { name: name.into() }),
            }
        }

        /// A helper.
        fn size(&self) -> u32 {
            2
        }

        /// An async helper.
        #[allow(dead_code)]
        async fn audit(&self) -> impl Stream<Item = String> + Send + 'static {
            stream! { yield "audited".to_string(); }
        }
    }
}

// ------------------------------------------------------------------ a marker still wins

/// A block under `served = written` whose markers say otherwise.
#[derive(Clone)]
pub struct OverrideActivation;

#[plexus_macros::activation(served = written)]
impl OverrideActivation {
    /// Private, but marked: served.
    #[plexus_macros::method]
    async fn private_but_marked(&self) -> u32 {
        1
    }

    /// Named on the wire otherwise.
    #[plexus_macros::method(name = "renamed")]
    pub async fn wire_name(&self) -> u32 {
        2
    }

    /// Private, but marked: a mount, with an option no signature carries.
    #[plexus_macros::child(opaque)]
    fn raw(&self) -> GreeterActivation {
        GreeterActivation
    }

    /// A slot whose names a sibling lists.
    #[plexus_macros::child(list = names)]
    pub fn book(&self, name: &str) -> Option<BookActivation> {
        (name == "dune").then(|| BookActivation { title: "Dune".into() })
    }

    /// The slot's names: `pub fn` on `&self`, but the marker's sibling — a
    /// helper, not a mount.
    pub fn names(&self) -> impl Stream<Item = String> + Send + '_ {
        stream! { yield "dune".to_string(); }
    }
}

// ------------------------------------------------------------------ helpers

fn schema_json<A: Activation>(a: &A) -> Value {
    serde_json::to_value(a.plugin_schema()).expect("a schema serializes")
}

fn method<'a>(schema: &'a Value, name: &str) -> &'a Value {
    schema["methods"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["name"] == name)
        .unwrap_or_else(|| panic!("no method {name} in {schema:#}"))
}

fn method_names(schema: &Value) -> Vec<String> {
    schema["methods"].as_array().unwrap().iter().map(|m| m["name"].as_str().unwrap().to_string()).collect()
}

async fn ir_of<A: Activation + ChildRouter + Clone>(a: A) -> Value {
    let hub = DynamicHub::new("svc").register(a);
    serde_json::to_value(hub.service_ir(&[], None).await.expect("the IR")).unwrap()
}

/// Everything a client of the shelf can do, written once and run against
/// each shelf's own generated client: the calls it sends, in order.
macro_rules! drive_client {
    ($module:ident) => {{
        let t = MemoryTransport::new();
        let c = $module::ShelfClient::new(&t);
        let _ = c.titles().collect::<Vec<_>>().await;
        let _ = c.count().await;
        let _ = c.rename("dune".into(), "arrakis".into()).await;
        let _ = c.index().entries().collect::<Vec<_>>().await;
        let _ = c.book("dune").title().collect::<Vec<_>>().await;
        let _ = c.greeter("here").call("hello", json!({ "name": "x" })).collect::<Vec<_>>().await;
        assert_eq!(c.index().path(), "shelf.index");
        assert_eq!(c.book("dune").path(), "shelf.dune");
        assert_eq!(c.greeter("here").path(), "shelf.greeter.here");
        t.calls()
    }};
}

/// The same, served: what each call answers through the router.
macro_rules! drive_served {
    ($module:ident) => {{
        let t = HubTransport::anonymous(Arc::new(DynamicHub::new("svc").register($module::ShelfActivation::new())));
        let c = $module::ShelfClient::new(t);
        let mut out: Vec<String> = Vec::new();
        out.push(format!("{:?}", c.titles().collect::<Vec<_>>().await));
        out.push(format!("{:?}", c.count().await));
        out.push(format!("{:?}", c.rename("dune".into(), "arrakis".into()).await));
        out.push(format!("{:?}", c.rename("emma".into(), "x".into()).await));
        out.push(format!("{:?}", c.index().entries().collect::<Vec<_>>().await));
        out.push(format!("{:?}", c.book("dune").title().collect::<Vec<_>>().await));
        out.push(format!("{:?}", c.greeter("here").call("hello", json!({ "name": "x" })).collect::<Vec<_>>().await));
        out
    }};
}

// ------------------------------------------------------------------ acceptance 1: the same, either way

#[tokio::test]
async fn the_service_ir_is_the_same_written_either_way() {
    let marked = ir_of(marked::ShelfActivation::new()).await;
    let written = ir_of(written::ShelfActivation::new()).await;
    assert_eq!(marked, written);
}

#[test]
fn the_schema_and_its_grants_are_the_same_written_either_way() {
    let marked = schema_json(&marked::ShelfActivation);
    let written = schema_json(&written::ShelfActivation);
    assert_eq!(marked, written);
    assert_eq!(marked::ShelfActivation.plugin_schema().roles, written::ShelfActivation.plugin_schema().roles);
    assert_eq!(
        json!(written::ShelfActivation.plugin_schema().roles),
        json!({
            "viewer": ["shelf.titles", "shelf.count", "shelf.{book}.title"],
            "editor": ["shelf.titles", "shelf.count", "shelf.{book}.title", "shelf.rename"],
        })
    );
    assert_eq!(Activation::methods(&marked::ShelfActivation), Activation::methods(&written::ShelfActivation));
}

#[tokio::test]
async fn the_client_is_the_same_written_either_way() {
    let marked = drive_client!(marked);
    let written = drive_client!(written);
    assert_eq!(marked, written);
    let names: Vec<&str> = written.iter().map(|(m, _)| m.as_str()).collect();
    assert_eq!(
        names,
        vec!["shelf.titles", "shelf.count", "shelf.rename", "shelf.index.entries", "shelf.dune.title", "shelf.greeter.here.hello"]
    );
}

#[tokio::test]
async fn what_is_served_answers_the_same_written_either_way() {
    let marked = drive_served!(marked);
    let written = drive_served!(written);
    assert_eq!(marked, written);
    assert_eq!(written[0], r#"[Ok("2 titles")]"#);
    assert_eq!(written[5], r#"[Ok("Dune")]"#);
}

// ------------------------------------------------------------------ acceptance 2: each row

#[test]
fn a_fn_that_is_not_pub_is_a_helper() {
    let schema = schema_json(&written::ShelfActivation);
    let names = method_names(&schema);
    assert!(!names.iter().any(|n| n == "size" || n == "audit"), "{names:?}");
}

#[test]
fn a_pub_fn_not_on_self_is_a_helper() {
    let names = method_names(&schema_json(&written::ShelfActivation));
    assert!(!names.iter().any(|n| n == "new"), "{names:?}");
}

#[test]
fn any_other_pub_async_fn_is_a_method() {
    let schema = schema_json(&written::ShelfActivation);
    for (name, cardinality) in [("titles", "many"), ("count", "one"), ("rename", "one")] {
        let m = method(&schema, name);
        assert_eq!(m["role"]["kind"], "rpc", "{name}: {m:#}");
        assert_eq!(m["cardinality"], cardinality, "{name}: {m:#}");
    }
}

#[tokio::test]
async fn a_pub_fn_on_self_is_a_static_mount() {
    let schema = schema_json(&written::ShelfActivation);
    assert_eq!(method(&schema, "index")["role"]["kind"], "static_child");
    assert!(written::ShelfActivation.get_child("index").await.is_some());
}

#[tokio::test]
async fn a_pub_fn_taking_a_name_is_a_slot() {
    let schema = schema_json(&written::ShelfActivation);
    assert_eq!(method(&schema, "book")["role"]["kind"], "dynamic_child");
    assert!(written::ShelfActivation.get_child("dune").await.is_some());
    assert!(written::ShelfActivation.get_child("emma").await.is_none());
}

#[test]
fn a_pub_async_fn_answering_a_boxed_interface_is_a_fallible_mount() {
    let schema = schema_json(&written::ShelfActivation);
    let m = method(&schema, "greeter");
    assert_eq!(m["role"], method(&schema_json(&marked::ShelfActivation), "greeter")["role"]);
    // On the wire a peer is a static child marked `peer` (NEST-7).
    assert_eq!(m["role"], json!({ "kind": "static_child", "peer": true }), "{m:#}");
}

#[tokio::test]
async fn a_marker_says_what_the_fn_is_whatever_its_signature() {
    let schema = schema_json(&OverrideActivation);
    let names = method_names(&schema);
    // Private and marked: served. A wire name the signature cannot carry.
    assert!(names.contains(&"private_but_marked".to_string()), "{names:?}");
    assert!(names.contains(&"renamed".to_string()) && !names.contains(&"wire_name".to_string()), "{names:?}");
    // `#[child(opaque)]` on a private fn: a mount with no typed client.
    assert_eq!(method(&schema, "raw")["role"]["kind"], "static_child");
    assert!(OverrideActivation.get_child("raw").await.is_some());
    let t = MemoryTransport::new();
    let client = OverrideClient::new(&t);
    let raw: plexus_client::Untyped<&&MemoryTransport> = client.raw();
    assert_eq!(raw.path(), "override.raw");
    // `#[child(list = names)]`: `names` is the marker's sibling, not a mount.
    assert!(!names.contains(&"names".to_string()), "{names:?}");
    assert!(OverrideActivation.get_child("names").await.is_none());
    let listed: Vec<String> = OverrideActivation.list_children().await.expect("a list").collect().await;
    assert_eq!(listed, vec!["raw".to_string(), "dune".to_string()]);
}

// ------------------------------------------------------------------ the compile errors

#[test]
fn keel27_compile_pass() {
    let t = trybuild::TestCases::new();
    t.pass("tests/compile/keel27_generic_mounts_ok.rs");
}

#[test]
fn keel27_compile_fail() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile/keel27_mount_not_an_activation.rs");
    t.compile_fail("tests/compile/keel27_slot_holds_no_activation.rs");
    t.compile_fail("tests/compile/keel27_slot_not_an_option.rs");
    t.compile_fail("tests/compile/keel27_mount_takes_an_argument.rs");
    t.compile_fail("tests/compile/keel27_fallible_mount_without_a_name.rs");
    t.compile_fail("tests/compile/keel27_served_written_and_methods_marked.rs");
}
