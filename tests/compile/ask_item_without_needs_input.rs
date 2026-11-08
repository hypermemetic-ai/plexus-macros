//! ASK-2 compile-fail: a method marked `asks` yields its proposal as its
//! item type's `NeedsInput(NeedsInput)` variant.

use plexus_core::plexus::ConfirmToken;

#[derive(Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Ev {
    Wiped,
}

#[derive(Clone)]
struct Wipe;

#[plexus_macros::activation(namespace = "wipe", crate_path = "plexus_core")]
impl Wipe {
    #[plexus_macros::method(asks)]
    async fn wipe(&self, confirm: Option<ConfirmToken>) -> Ev {
        let _ = confirm;
        Ev::Wiped
    }
}

fn main() {}
