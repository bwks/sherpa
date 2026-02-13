use crate::api::websocket::connection::ConnectionRegistry;

/// Application state shared across the server.
///
/// This contains all runtime state needed by handlers, including:
/// - WebSocket connection registry for real-time communication
/// - (Future) Database connections, libvirt/Docker clients, etc.
#[derive(Clone)]
pub struct AppState {
    /// Registry of active WebSocket connections
    pub connections: ConnectionRegistry,
}

impl AppState {
    /// Create a new AppState with empty connection registry
    pub fn new() -> Self {
        Self {
            connections: crate::api::websocket::connection::create_registry(),
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
