use futures::StreamExt;
use macro13_no_client::{Echo, EchoActivation, EchoEvent};
use plexus_core::plexus::Activation;

#[tokio::test]
async fn an_activation_without_plexus_client_builds_without_a_client() {
    assert_eq!(EchoActivation.namespace(), "echo");
    let echo: Box<dyn Echo> = Box::new(EchoActivation);
    let got: Vec<EchoEvent> = echo.say("hi".into()).collect().await;
    assert_eq!(got, vec![EchoEvent::Said("hi".into())]);
}
