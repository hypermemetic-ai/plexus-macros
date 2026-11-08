//! ASK-2 compile-fail: `bidirectional` is removed; the error says what replaced it.

#[derive(Clone)]
struct Wizard;

#[plexus_macros::activation(namespace = "wizard", crate_path = "plexus_core")]
impl Wizard {
    #[plexus_macros::method(bidirectional)]
    async fn ask(&self) -> String {
        String::new()
    }
}

fn main() {}
