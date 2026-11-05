//! RETURNS-47: a bodiless activation is a contract, and a contract is its
//! trait; on a type with no `…Activation` suffix the trait cannot share the
//! type's name, so it must be named.

#[plexus_macros::activation]
impl Driver {
    pub async fn describe(&self) -> impl futures::Stream<Item = String> + Send + 'static;
}

fn main() {}
