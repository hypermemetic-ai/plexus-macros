//! RETURNS-47: on `impl Trait for Type` the trait is the one written; a
//! `trait = "…"` beside it names a second, and is refused.

#[allow(async_fn_in_trait)]
pub trait Forge {
    async fn total(&self) -> u64;
}

#[derive(Clone)]
pub struct GitHub;

#[plexus_macros::activation(trait = "Forge2")]
impl Forge for GitHub {
    async fn total(&self) -> u64 {
        3
    }
}

fn main() {}
