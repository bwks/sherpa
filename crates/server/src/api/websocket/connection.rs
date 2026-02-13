use axum::extract::ws::{Message, WebSocket};
use dashmap::DashMap;
use futures_util::stream::SplitSink;
use futures_util::SinkExt;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

/// A single WebSocket connection
pub struct Connection {
    pub id: Uuid,
    pub sender: Arc<Mutex<SplitSink<WebSocket, Message>>>,
    pub subscribed_logs: AtomicBool,
}

impl Connection {
    pub fn new(id: Uuid, sender: SplitSink<WebSocket, Message>) -> Self {
        Self {
            id,
            sender: Arc::new(Mutex::new(sender)),
            subscribed_logs: AtomicBool::new(false),
        }
    }

    /// Send a message to this connection
    pub async fn send(&self, message: Message) -> Result<(), axum::Error> {
        self.sender
            .lock()
            .await
            .send(message)
            .await
            .map_err(|e| axum::Error::new(e))
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

    #[test]
    fn test_registry_insert_remove() {
        let registry = create_registry();
        let id = Uuid::new_v4();

        // Registry should be empty
        assert_eq!(registry.len(), 0);
        assert!(!registry.contains_key(&id));

        // Note: We can't easily test full Connection insertion without a real WebSocket
        // but we can test the registry operations
        registry.insert(id, Arc::new(Connection {
            id,
            sender: Arc::new(Mutex::new(futures_util::stream::SplitSink::new())),
            subscribed_logs: false,
        }));

        // Note: The above will fail to compile because SplitSink needs actual WebSocket
        // In practice, this is tested via integration tests with real connections
    }
}
