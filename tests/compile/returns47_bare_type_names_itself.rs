//! RETURNS-47: a type not named `…Activation`, with nothing in the
//! attribute, is named by itself — namespace `driver`, client `DriverClient`.
//! (MACRO-13 refused it.)

use futures::Stream;
use plexus_client::MemoryTransport;
use plexus_core::plexus::Activation;

#[derive(Clone)]
pub struct Driver;

#[plexus_macros::activation]
impl Driver {
    pub async fn describe(&self) -> impl Stream<Item = String> + Send + 'static {
        futures::stream::empty()
    }
}

fn main() {
    assert_eq!(Driver.namespace(), "driver");
    assert_eq!(DriverClient::<MemoryTransport>::NAMESPACE, "driver");
}
