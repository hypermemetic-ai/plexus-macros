//! BIDIR-6 compile-fail: an `Input` in a param's struct. The macro sees only
//! the method's signature, so it cannot name the method here; `Input` takes
//! no serde, so the struct does not compile.
use plexus_core::plexus::Input;

#[derive(serde::Deserialize)]
struct Bag {
    tokens: Input<u32>,
}

fn main() {}
