//! MACRO-12 negative fixture: under `typed_params`, every bare string
//! parameter is a compile error on its own span, all reported at once.

use futures::Stream;

pub struct Tickets;

#[plexus_macros::activation(namespace = "tickets", typed_params, crate_path = "plexus_core")]
impl Tickets {
    #[plexus_macros::method]
    async fn get(&self, id: String) -> impl Stream<Item = String> + Send + 'static {
        let _ = id;
        futures::stream::empty()
    }

    #[plexus_macros::method]
    async fn list(
        &self,
        epic: Option<String>,
        ids: Vec<String>,
        filter: std::string::String,
    ) -> impl Stream<Item = String> + Send + 'static {
        let _ = (epic, ids, filter);
        futures::stream::empty()
    }

    #[plexus_macros::method]
    async fn find(&self, query: &str, near: Option<&str>) -> impl Stream<Item = String> + Send + 'static {
        let _ = (query.to_string(), near.map(str::to_string));
        futures::stream::empty()
    }
}

fn main() {}
