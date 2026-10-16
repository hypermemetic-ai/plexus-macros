//! MACRO-13: a contract has no bodies and a server has all of them; a mix is
//! named at its first bodiless method.

use futures::Stream;

#[derive(Clone)]
pub struct MixedActivation;

#[plexus_macros::activation]
impl MixedActivation {
    pub async fn served(&self) -> impl Stream<Item = String> + Send + 'static {
        futures::stream::empty()
    }

    pub async fn promised(&self) -> impl Stream<Item = String> + Send + 'static;
}

fn main() {}
