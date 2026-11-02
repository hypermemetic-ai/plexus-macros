//! RETURNS-36: `#[method(streaming)]` is a deprecated no-op where the
//! signature agrees, and an error where it does not — here, a one-item form.

pub struct TickerActivation;

#[plexus_macros::activation]
impl TickerActivation {
    /// Ticks once.
    #[plexus_macros::method(streaming)]
    async fn tick(&self) -> u64 {
        7
    }
}

fn main() {}
