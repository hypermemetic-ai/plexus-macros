//! PLX-79: the capability handle's TYPE is the callback declaration, so a
//! `Client` written without its capability set declares nothing. That is an
//! error at the handle, not a silently empty callback list.

fn main() {
    let _ = plexus_macros::method_ir!(
        namespace = "ns",
        #[method]
        async fn f(&self, client: Client) -> Result<String, String>;
    );
}
