//! RETURNS-40: the fixture as a service's `conformance` child, ungated, on
//! a WebSocket — `synapse --url ws://127.0.0.1:PORT -i conformance_service`
//! is answered by one `conformance_service.ir`.
//! `cargo run -p conformance --example service -- PORT`.

use std::sync::Arc;

use conformance::ConformanceActivation;
use plexus_core::DynamicHub;
use plexus_transport::TransportServer;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let port: u16 = std::env::args().nth(1).ok_or_else(|| anyhow::anyhow!("usage: serve PORT"))?.parse()?;
    let service = Arc::new(DynamicHub::new("conformance_service").register(ConformanceActivation));
    TransportServer::builder(service, |h| Ok(DynamicHub::arc_into_rpc_module(h)?))
        .without_registry()
        .with_websocket(port)
        .with_allowed_hosts([format!("127.0.0.1:{port}"), format!("localhost:{port}")])
        .build()
        .await?
        .serve()
        .await
}
