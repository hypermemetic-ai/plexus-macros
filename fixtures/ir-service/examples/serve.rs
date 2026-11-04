//! RETURNS-40: the fixture's service on a WebSocket, ungated, for a client
//! to ask — `synapse --url ws://127.0.0.1:PORT -i irsvc` is answered by one
//! `irsvc.ir`. `cargo run -p ir-service --example serve -- PORT`;
//! `RUST_LOG=jsonrpsee_server=debug` logs each call it is sent.

use std::sync::Arc;

use plexus_core::DynamicHub;
use plexus_transport::TransportServer;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).with_writer(std::io::stderr).init();
    let port: u16 = std::env::args().nth(1).ok_or_else(|| anyhow::anyhow!("usage: serve PORT"))?.parse()?;
    let service = Arc::new(ir_service::service());
    TransportServer::builder(service, |h| Ok(DynamicHub::arc_into_rpc_module(h)?))
        .without_registry()
        .with_websocket(port)
        .with_allowed_hosts([format!("127.0.0.1:{port}"), format!("localhost:{port}")])
        .build()
        .await?
        .serve()
        .await
}
