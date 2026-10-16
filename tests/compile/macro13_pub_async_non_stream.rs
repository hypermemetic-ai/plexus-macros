//! MACRO-13: every `pub async fn` on `&self` is a wire method, so one that
//! does not return the event stream is named.

use futures::Stream;

#[derive(Clone)]
pub struct CounterActivation;

#[plexus_macros::activation]
impl CounterActivation {
    pub async fn watch(&self) -> impl Stream<Item = String> + Send + 'static {
        futures::stream::empty()
    }

    pub async fn total(&self) -> u32 {
        7
    }
}

fn main() {}
