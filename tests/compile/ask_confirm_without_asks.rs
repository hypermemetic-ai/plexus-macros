//! ASK-2 compile-fail: a `confirm` token on a method not marked `asks`.

use plexus_core::plexus::ConfirmToken;

#[derive(Clone)]
struct Wipe;

#[plexus_macros::activation(namespace = "wipe", crate_path = "plexus_core")]
impl Wipe {
    #[plexus_macros::method]
    async fn wipe(&self, what: String, confirm: Option<ConfirmToken>) -> String {
        let _ = confirm;
        what
    }
}

fn main() {}
