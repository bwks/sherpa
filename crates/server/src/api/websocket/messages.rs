use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use shared::data::StatusKind;
use shared::error::RpcErrorCode;
use std::collections::HashMap;

/// RPC error information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcError {
    /// JSON-RPC error code
    pub code: RpcErrorCode,
    /// Human-readable error message
    pub message: String,
    /// Additional debug context (file:line, stack trace)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
}

/// Message sent from server to client
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    /// Log message
    #[allow(dead_code)]
    Log {
        level: LogLevel,
        message: String,
        timestamp: Timestamp,
        #[serde(skip_serializing_if = "Option::is_none")]
        context: Option<HashMap<String, String>>,
    },

    /// Status update
    Status {
        message: String,
        timestamp: Timestamp,
        kind: StatusKind,
        #[serde(skip_serializing_if = "Option::is_none")]
        phase: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        progress: Option<StatusProgress>,
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
    #[allow(dead_code)]
    Ping,

    /// RPC response
    RpcResponse {
        id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        result: Option<serde_json::Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<RpcError>,
    },
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

/// Progress information for status updates
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusProgress {
    pub current_phase: String,
    pub phase_number: u8,
    pub total_phases: u8,
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

    /// RPC request
    RpcRequest {
        id: String,
        method: String,
        params: serde_json::Value,
    },
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
            timestamp: Timestamp::now(),
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

    #[test]
    fn test_rpc_request_deserialization() {
        let json = r#"{"type":"rpc_request","id":"test-001","method":"inspect","params":{"lab_id":"abc123"}}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            ClientMessage::RpcRequest { id, method, params } => {
                assert_eq!(id, "test-001");
                assert_eq!(method, "inspect");
                assert_eq!(params.get("lab_id").unwrap().as_str().unwrap(), "abc123");
            }
            _ => panic!("Expected RpcRequest"),
        }
    }

    #[test]
    fn test_rpc_response_success_serialization() {
        let msg = ServerMessage::RpcResponse {
            id: "test-001".to_string(),
            result: Some(serde_json::json!({"status": "ok"})),
            error: None,
        };

        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"rpc_response\""));
        assert!(json.contains("\"id\":\"test-001\""));
        assert!(json.contains("\"result\""));
        assert!(!json.contains("\"error\""));
    }

    #[test]
    fn test_status_message_serialization_with_kind() {
        let msg = ServerMessage::Status {
            message: "Creating VM".to_string(),
            timestamp: Timestamp::now(),
            kind: StatusKind::Progress,
            phase: None,
            progress: None,
        };

        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"status\""));
        assert!(json.contains("\"kind\":\"progress\""));
        assert!(json.contains("\"message\":\"Creating VM\""));
    }

    #[test]
    fn test_status_message_done_kind() {
        let msg = ServerMessage::Status {
            message: "All nodes are ready!".to_string(),
            timestamp: Timestamp::now(),
            kind: StatusKind::Done,
            phase: Some("Node Readiness Check".to_string()),
            progress: Some(StatusProgress {
                current_phase: "Node Readiness Check".to_string(),
                phase_number: 13,
                total_phases: 13,
            }),
        };

        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"kind\":\"done\""));
        assert!(json.contains("\"phase\":\"Node Readiness Check\""));
    }

    #[test]
    fn test_rpc_response_error_serialization() {
        let msg = ServerMessage::RpcResponse {
            id: "test-002".to_string(),
            result: None,
            error: Some(RpcError {
                code: RpcErrorCode::InternalError,
                message: "Internal error".to_string(),
                context: Some("file.rs:123".to_string()),
            }),
        };

        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"rpc_response\""));
        assert!(json.contains("\"id\":\"test-002\""));
        assert!(json.contains("\"error\""));
        assert!(json.contains("\"code\":-32603"));
        assert!(json.contains("\"message\":\"Internal error\""));
    }
}
