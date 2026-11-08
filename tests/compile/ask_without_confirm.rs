//! ASK-2 compile-fail: a method marked `asks` takes `confirm: Option<ConfirmToken>`.

use plexus_core::plexus::NeedsInput;

#[derive(Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Ev {
    NeedsInput(NeedsInput),
    Done,
}

#[derive(Clone)]
struct Wipe;

#[plexus_macros::activation(namespace = "wipe", crate_path = "plexus_core")]
impl Wipe {
    #[plexus_macros::method(asks)]
    async fn wipe(&self, what: String) -> Ev {
        let _ = what;
        Ev::Done
    }
}

fn main() {}
