//! MACRO-13: every `pub async fn` on `&self` is a wire method, so one that
//! returns no signature form is named. (RETURNS-36: `-> u32` is now a form —
//! one item, RETURNS-8 — so the case is a boxed stream, which is not.)

use futures::Stream;

#[derive(Clone)]
pub struct CounterActivation;

#[plexus_macros::activation]
impl CounterActivation {
    pub async fn watch(&self) -> impl Stream<Item = String> + Send + 'static {
        futures::stream::empty()
    }

    pub async fn total(&self) -> futures::stream::BoxStream<'static, u32> {
        Box::pin(futures::stream::empty())
    }
}

fn main() {}
