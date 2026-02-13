use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Message sent from server to client
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    /// Log message
    Log {
        level: LogLevel,
        message: String,
        timestamp: DateTime<Utc>,
        #[serde(skip_serializing_if = "Option::is_none")]
        context: Option<HashMap<String, String>>,
    },

    /// Status update
    Status {
        message: String,
        timestamp: DateTime<Utc>,
    },

    /// Operation result
    Result {
        success: bool,
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        data: Option<serde_json::Value>,
    },

    /// Connection established confirmation
    Connected { connection_id: String },

    /// Ping for keepalive
    Ping,
}

/// Log levels matching tracing levels
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

/// Message sent from client to server
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    /// Subscribe to log stream
    SubscribeLogs,

    /// Unsubscribe from log stream
    UnsubscribeLogs,

    /// Ping response
    Pong,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_server_message_serialization() {
        let msg = ServerMessage::Connected {
            connection_id: "test-123".to_string(),
        };

        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"connected\""));
        assert!(json.contains("\"connection_id\":\"test-123\""));
    }

    #[test]
    fn test_log_message_serialization() {
        let msg = ServerMessage::Log {
            level: LogLevel::Info,
            message: "Test log".to_string(),
            timestamp: Utc::now(),
            context: None,
        };

        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"log\""));
        assert!(json.contains("\"level\":\"info\""));
        assert!(json.contains("\"message\":\"Test log\""));
    }

    #[test]
    fn test_client_message_deserialization() {
        let json = r#"{"type":"subscribe_logs"}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();
        assert!(matches!(msg, ClientMessage::SubscribeLogs));
    }

    #[test]
    fn test_log_level_serialization() {
        let level = LogLevel::Warn;
        let json = serde_json::to_string(&level).unwrap();
        assert_eq!(json, "\"warn\"");
    }
}
