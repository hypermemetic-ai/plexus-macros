//! BIDIR-8 compile-fail: `answer_by` is one of three spellings.

#[derive(Clone)]
struct Runs;

#[plexus_macros::activation(namespace = "runs", crate_path = "plexus_core")]
impl Runs {
    #[plexus_macros::method(answer_by = "anyone")]
    async fn approve(&self, run: String) -> impl futures::stream::Stream<Item = String> + Send + 'static {
        async_stream::stream! { yield run; }
    }
}

fn main() {}
