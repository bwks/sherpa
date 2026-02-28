use super::connection::ConnectionRegistry;
use super::messages::ServerMessage;
use axum::extract::ws::Message;
use std::sync::atomic::Ordering;

/// Broadcast a message to all connected clients
pub async fn _broadcast_to_all(registry: &ConnectionRegistry, message: &ServerMessage) -> usize {
    let json = match serde_json::to_string(message) {
        Ok(j) => j,
        Err(e) => {
            tracing::error!("Failed to serialize message: {}", e);
            return 0;
        }
    };

    let msg = Message::Text(json.into());
    let mut sent_count = 0;

    for entry in registry.iter() {
        let conn = entry.value();
        if let Err(e) = conn.send(msg.clone()).await {
            tracing::warn!("Failed to send to connection {}: {}", conn.id, e);
        } else {
            sent_count += 1;
        }
    }

    tracing::debug!("Broadcasted message to {} connections", sent_count);
    sent_count
}

/// Broadcast a log message to subscribed clients only
pub async fn _broadcast_log(registry: &ConnectionRegistry, message: &ServerMessage) -> usize {
    let json = match serde_json::to_string(message) {
        Ok(j) => j,
        Err(e) => {
            tracing::error!("Failed to serialize log message: {}", e);
            return 0;
        }
    };

    let msg = Message::Text(json.into());
    let mut sent_count = 0;

    for entry in registry.iter() {
        let conn = entry.value();

        // Only send to clients subscribed to logs
        if conn.subscribed_logs.load(Ordering::Relaxed) {
            if let Err(e) = conn.send(msg.clone()).await {
                tracing::warn!("Failed to send log to {}: {}", conn.id, e);
            } else {
                sent_count += 1;
            }
        }
    }

    if sent_count > 0 {
        tracing::debug!("Broadcasted log to {} subscribers", sent_count);
    }

    sent_count
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::websocket::messages::LogLevel;
    use jiff::Timestamp;

    #[tokio::test]
    async fn test_broadcast_to_empty_registry() {
        let registry = crate::api::websocket::connection::create_registry();

        let message = ServerMessage::Status {
            message: "Test".to_string(),
            timestamp: Timestamp::now(),
            kind: shared::data::StatusKind::Info,
            phase: None,
            progress: None,
        };

        let count = _broadcast_to_all(&registry, &message).await;
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test__broadcast_log_to_empty_registry() {
        let registry = crate::api::websocket::connection::create_registry();

        let message = ServerMessage::Log {
            level: LogLevel::Info,
            message: "Test log".to_string(),
            timestamp: jiff::Timestamp::now(),
            context: None,
        };

        let count = _broadcast_log(&registry, &message).await;
        assert_eq!(count, 0);
    }
}
