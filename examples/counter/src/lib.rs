use futures::{stream, Stream, StreamExt};
use plexus_macros::PlexusWire;

#[derive(Debug, Clone, PartialEq, PlexusWire)]
pub enum CountEvent {
    Tick { n: u32 },
}

#[derive(Clone, Default)]
pub struct Counter;

/// Counts.
#[plexus_macros::activation]
impl Counter {
    /// Ticks from 1 up to `to`.
    #[plexus_macros::method]
    pub async fn count(&self, /** the last tick */ to: u32) -> impl Stream<Item = CountEvent> {
        stream::iter(1..=to).map(|n| CountEvent::Tick { n })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::TryStreamExt;
    use plexus_core::plexus::{Activation, InProcessTransport};

    #[tokio::test]
    async fn the_client_counts() {
        let client = CounterClient::new(InProcessTransport::new(Counter));
        let ticks: Vec<CountEvent> = client.count(3).try_collect().await.unwrap();
        assert_eq!(ticks, [1, 2, 3].map(|n| CountEvent::Tick { n }));
    }

    #[test]
    fn the_activation_describes_itself() {
        let schema = serde_json::to_value(Counter.plugin_schema()).unwrap();
        assert_eq!(schema["namespace"], "counter");
        assert_eq!(schema["description"], "Counts.");
        let count = schema["methods"].as_array().unwrap().iter().find(|m| m["name"] == "count").unwrap();
        assert_eq!(count["description"], "Ticks from 1 up to `to`.");
    }
}
