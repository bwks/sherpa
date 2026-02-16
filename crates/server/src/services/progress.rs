use anyhow::Result;
use axum::extract::ws::Message;
use jiff::Timestamp;
use tokio::sync::mpsc;

use crate::api::websocket::messages::{ServerMessage, StatusProgress};
use shared::data::UpPhase;

/// Progress sender for streaming updates during long operations
#[derive(Clone)]
pub struct ProgressSender {
    tx: mpsc::UnboundedSender<Message>,
}

impl ProgressSender {
    /// Create a new progress sender
    pub fn new(tx: mpsc::UnboundedSender<Message>) -> Self {
        Self { tx }
    }

    /// Send a phase progress update
    pub fn send_phase(&self, phase: UpPhase, message: String) -> Result<()> {
        let server_msg = ServerMessage::Status {
            message: message.clone(),
            timestamp: Timestamp::now(),
            phase: Some(phase.as_str().to_string()),
            progress: Some(StatusProgress {
                current_phase: phase.as_str().to_string(),
                phase_number: phase.number(),
                total_phases: UpPhase::total_phases(),
            }),
        };

        let json = serde_json::to_string(&server_msg)?;
        self.tx.send(Message::Text(json.into()))?;
        Ok(())
    }

    /// Send a simple status message (without phase info)
    pub fn send_status(&self, message: String) -> Result<()> {
        let server_msg = ServerMessage::Status {
            message,
            timestamp: Timestamp::now(),
            phase: None,
            progress: None,
        };

        let json = serde_json::to_string(&server_msg)?;
        self.tx.send(Message::Text(json.into()))?;
        Ok(())
    }
}
