//! PLX-71 compile-fail: a duplicate key within one `ext(...)` is rejected at
//! compile time with a `syn::Error` naming the duplicated key.

#[derive(Clone)]
struct DupExt;

#[plexus_macros::activation(namespace = "dup_ext", crate_path = "plexus_core")]
impl DupExt {
    #[plexus_macros::method(ext(nostr_kind = 30023, nostr_kind = 30024))]
    async fn broken(&self) -> impl futures::stream::Stream<Item = String> + Send + 'static {
        async_stream::stream! { yield "x".into(); }
    }
}

fn main() {}
