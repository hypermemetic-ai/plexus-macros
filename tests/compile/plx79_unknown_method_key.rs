//! PLX-79 acceptance criterion #3: an unrecognized key inside `#[method(...)]`
//! is a compile error that NAMES the key.
//!
//! vNext reverses the old path's "ignore unknown keys for forward
//! compatibility" posture — `ext(...)` is the escape hatch, so a typo is a
//! typo and must fail loudly (PLX-73 trap closures; M1-T6).

fn main() {
    let _ = plexus_macros::method_ir!(
        namespace = "ns",
        #[method(unknwon = 1)]
        async fn f(&self) -> Result<String, String>;
    );
}
