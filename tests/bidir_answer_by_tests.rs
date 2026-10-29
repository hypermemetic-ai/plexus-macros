//! BIDIR-8: `#[plexus_macros::method(answer_by = "…")]` — who may answer a
//! question the method asks mid-call (BIDIR-1 `answerer`): `caller` (the
//! default), `person`, or `grant:<activation.method>`. It lands in the
//! schema as `MethodSchema.answer_by`, absent for the default so a method
//! without it is byte-identical; any other spelling is a compile error
//! naming the method and the three spellings
//! (`tests/compile/bidir_answer_by_unknown.rs`).
//!
//! Run with: `cargo test --test bidir_answer_by_tests`.

use async_stream::stream;
use futures::stream::Stream;
use plexus_core::plexus::Activation;

#[derive(Clone)]
struct RunsActivation;

#[plexus_macros::activation(
    version = "1.0.0",
    crate_path = "plexus_core",
    roles(editor = ["approve", "release", "plain", "said_caller"]),
)]
impl RunsActivation {
    /// A step only a person may say yes to.
    #[plexus_macros::method(target = "run", answer_by = "person")]
    async fn approve(&self, run: String) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield run; }
    }

    /// A release whoever holds `runs.release_approve` on the run may answer.
    #[plexus_macros::method(target = "run", answer_by = "grant:runs.release_approve")]
    async fn release(&self, run: String) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield run; }
    }

    /// No declaration: the caller answers.
    #[plexus_macros::method(target = "run")]
    async fn plain(&self, run: String) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield run; }
    }

    /// The default, written out.
    #[plexus_macros::method(answer_by = "caller")]
    async fn said_caller(&self, run: String) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield run; }
    }
}

fn method(name: &str) -> plexus_core::plexus::MethodSchema {
    RunsActivation.plugin_schema().methods.into_iter().find(|m| m.name == name).unwrap_or_else(|| panic!("no method {name}"))
}

#[test]
fn answer_by_lands_in_the_schema() {
    assert_eq!(method("approve").answer_by.as_deref(), Some("person"));
    assert_eq!(method("release").answer_by.as_deref(), Some("grant:runs.release_approve"));
    // And reads back as the gate's Answerer.
    assert_eq!(method("approve").answerer(), Ok(plexus_auth_core::duplex::Answerer::Person));
    assert_eq!(
        method("release").answerer(),
        Ok(plexus_auth_core::duplex::Answerer::Grant("runs.release_approve".into()))
    );
    let json = serde_json::to_value(method("approve")).unwrap();
    assert_eq!(json["answer_by"], "person");
}

#[test]
fn default_is_absent_from_the_schema() {
    for name in ["plain", "said_caller"] {
        let m = method(name);
        assert_eq!(m.answer_by, None, "{name}");
        let json = serde_json::to_value(&m).unwrap();
        assert!(json.get("answer_by").is_none(), "{name}: byte-identical on the wire: {json}");
        assert_eq!(m.answerer(), Ok(plexus_auth_core::duplex::Answerer::Caller));
    }
    // Declaring the default changes no hash: `caller` is what absent means.
    assert_eq!(method("plain").answer_by, method("said_caller").answer_by);
}

#[test]
fn answer_by_unknown_is_a_compile_error() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile/bidir_answer_by_unknown.rs");
}
