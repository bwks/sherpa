use axum::extract::ws::{Message, WebSocket};
use dashmap::DashMap;
use futures_util::SinkExt;
use futures_util::stream::SplitSink;
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

/// A single WebSocket connection
pub struct Connection {
    pub id: Uuid,
    pub sender: Arc<Mutex<SplitSink<WebSocket, Message>>>,
}

impl Connection {
    pub fn new(id: Uuid, sender: SplitSink<WebSocket, Message>) -> Self {
        Self {
            id,
            sender: Arc::new(Mutex::new(sender)),
        }
    }

    /// Send a message to this connection
    pub async fn send(&self, message: Message) -> Result<(), axum::Error> {
        self.sender
            .lock()
            .await
            .send(message)
            .await
            .map_err(axum::Error::new)
    }
}

/// Global connection registry
pub type ConnectionRegistry = Arc<DashMap<Uuid, Arc<Connection>>>;

/// Create a new connection registry
pub fn create_registry() -> ConnectionRegistry {
    Arc::new(DashMap::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_registry() {
        let registry = create_registry();
        assert_eq!(registry.len(), 0);
    }
}
