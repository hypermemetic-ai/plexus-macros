//! RETURNS-8 negative fixture: progress needs a stream (RETURNS-1 decision
//! 10) — `-> Progress<T>` is not a form.

pub struct Loader;

#[plexus_macros::activation]
impl Loader {
    /// Loads.
    #[plexus_macros::method]
    async fn load(&self) -> plexus_client::Progress<u64> {
        plexus_client::Progress::Item(1)
    }
}

fn main() {}
