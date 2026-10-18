//! Ctrl-C during a long-running lab operation (`up`, `redeploy`, `resume`).
//!
//! The server keeps an operation running when its client disconnects. On Ctrl-C the user is
//! asked whether to stop it on the server too; the default leaves it running.

use std::io::{self, Write};
use std::time::Duration;

use anyhow::{Context, Result};

use shared::data::{CancelLabResponse, ServerConnection};
use shared::util::Emoji;

use crate::ws_client::client::RpcClient;
use crate::ws_client::messages::RpcResponse;
use crate::ws_client::{RpcRequest, WebSocketClient};

/// Time allowed to connect to the server to stop an operation.
const CANCEL_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// A lab operation the user can stop with Ctrl-C.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoppableOperation {
    Up,
    Redeploy,
    Resume,
}

/// The server and lab an interrupted operation runs on.
pub struct OperationTarget<'a> {
    pub operation: StoppableOperation,
    pub lab_id: &'a str,
    pub token: &'a str,
    pub server_url: &'a str,
    pub server_connection: &'a ServerConnection,
}

/// How waiting for an operation ended.
pub enum Waited {
    /// The operation finished and the server sent its response.
    Response(RpcResponse),
    /// The user pressed Ctrl-C; the outcome has already been reported.
    Interrupted,
}

impl StoppableOperation {
    fn description(self) -> &'static str {
        match self {
            StoppableOperation::Up => "lab creation",
            StoppableOperation::Redeploy => "node redeploy",
            StoppableOperation::Resume => "lab resume",
        }
    }

    fn stop_effect(self) -> &'static str {
        match self {
            StoppableOperation::Up => "It stops at its next step and removes what it has created.",
            StoppableOperation::Redeploy => {
                "It stops at its next step; the node may be left half rebuilt."
            }
            StoppableOperation::Resume => {
                "It stops waiting for nodes; nodes already started keep running."
            }
        }
    }
}

/// Wait for a streaming lab operation, or for Ctrl-C.
pub async fn call_streaming_or_interrupt<F>(
    rpc_client: &mut RpcClient,
    request: RpcRequest,
    on_message: F,
    target: &OperationTarget<'_>,
) -> Result<Waited>
where
    F: FnMut(&str),
{
    tokio::select! {
        response = rpc_client.call_streaming(request, on_message) => Ok(Waited::Response(response?)),
        signal = tokio::signal::ctrl_c() => {
            signal.context("Failed to listen for Ctrl-C")?;
            handle_interrupt(target).await?;
            Ok(Waited::Interrupted)
        }
    }
}

/// Whether an answer to a `[y/N]` prompt asks to stop.
fn wants_stop(answer: &str) -> bool {
    matches!(answer.trim().to_lowercase().as_str(), "y" | "yes")
}

async fn handle_interrupt(target: &OperationTarget<'_>) -> Result<()> {
    let operation = target.operation.description();
    println!(
        "\n\n{} Interrupted. The {operation} is still running on the server.",
        Emoji::Warning
    );
    println!("   {}", target.operation.stop_effect());
    print!("Stop the {operation} on the server too? [y/N]: ");
    io::stdout().flush()?;
    let answer = tokio::task::spawn_blocking(read_answer)
        .await
        .context("Failed to read the answer")??;
    if !wants_stop(&answer) {
        println!(
            "{} The {operation} keeps running on the server. Check on it with `sherpa inspect`.",
            Emoji::Info
        );
        return Ok(());
    }

    println!(
        "{} Stopping the {operation}; waiting for it to finish its current step...",
        Emoji::Progress
    );
    let ws_client = WebSocketClient::new(
        target.server_url.to_string(),
        CANCEL_CONNECT_TIMEOUT,
        target.server_connection.clone(),
    );
    let mut rpc_client = ws_client
        .connect()
        .await
        .context("Failed to connect to sherpad server to stop the operation")?;
    let request = RpcRequest::new(
        "cancel",
        serde_json::json!({ "lab_id": target.lab_id, "token": target.token }),
    );
    let response = rpc_client
        .call(request)
        .await
        .context("Cancel RPC call failed")?;
    rpc_client.close().await.ok();

    if let Some(error) = response.error {
        eprintln!(
            "{} The server could not stop the {operation}: {}",
            Emoji::Error,
            error.message
        );
        if let Some(context) = error.context {
            eprintln!("   {context}");
        }
        return Ok(());
    }
    let result: CancelLabResponse = serde_json::from_value(
        response
            .result
            .context("No result in the cancel response")?,
    )
    .context("Failed to parse the cancel response")?;
    if result.stopped {
        println!("{} Stopped the {operation}.", Emoji::Success);
    } else {
        println!(
            "{} No operation was running on lab {}; it had already finished.",
            Emoji::Info,
            result.lab_id
        );
    }
    Ok(())
}

fn read_answer() -> Result<String> {
    let mut answer = String::new();
    io::stdin()
        .read_line(&mut answer)
        .context("Failed to read from the terminal")?;
    Ok(answer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_yes_stops_the_operation() {
        for answer in ["y", "Y", "yes", " YES \n"] {
            assert!(wants_stop(answer), "{answer:?}");
        }
        for answer in ["", "\n", "n", "no", "maybe", "yess"] {
            assert!(!wants_stop(answer), "{answer:?}");
        }
    }

    #[test]
    fn every_operation_says_what_stopping_does() {
        for operation in [
            StoppableOperation::Up,
            StoppableOperation::Redeploy,
            StoppableOperation::Resume,
        ] {
            assert!(!operation.description().is_empty());
            assert!(operation.stop_effect().ends_with('.'));
        }
        assert!(StoppableOperation::Up.stop_effect().contains("removes"));
    }
}
