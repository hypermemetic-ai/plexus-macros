//! ASK-1 compile-fail: `bidirectional` is removed; the error says so.

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
