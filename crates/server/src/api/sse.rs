use std::convert::Infallible;

use askama::Template;
use axum::extract::ws::Message;
use axum::response::sse::Event;
use futures::stream::Stream;
use serde::Serialize;
use shared::data::{DestroyResponse, StatusKind, UpResponse};
use tokio::sync::mpsc;

use crate::templates::{
    CreateSummaryFailedFragment, CreateSummarySuccessFragment, DestroyProgressLineFragment,
    DestroySummaryErrorsFragment, DestroySummaryFailedFragment, DestroySummarySuccessFragment,
};

/// Map a `StatusKind` to its corresponding emoji character.
fn status_emoji(kind: &StatusKind) -> &'static str {
    match kind {
        StatusKind::Progress => "\u{1F504}",
        StatusKind::Done => "\u{2705}",
        StatusKind::Info => "\u{2139}\u{FE0F}",
        StatusKind::Waiting => "\u{23F3}",
    }
}

/// Convert a progress message receiver and a destroy result receiver into an SSE
/// event stream suitable for consumption by HTMX's SSE extension.
///
/// The stream yields:
/// - `progress` events for each status message received on the channel
/// - A final `complete` event with a summary after the destroy operation finishes
pub fn destroy_progress_stream(
    mut rx: mpsc::UnboundedReceiver<Message>,
    result_rx: tokio::sync::oneshot::Receiver<anyhow::Result<DestroyResponse>>,
) -> impl Stream<Item = Result<Event, Infallible>> {
    async_stream::stream! {
        // Drain all progress messages from the channel
        while let Some(msg) = rx.recv().await {
            if let Message::Text(text) = msg
                && let Ok(server_msg) = serde_json::from_str::<serde_json::Value>(&text)
            {
                let kind_str = server_msg
                    .get("kind")
                    .and_then(|v| v.as_str())
                    .unwrap_or("info");
                let kind = match kind_str {
                    "progress" => StatusKind::Progress,
                    "done" => StatusKind::Done,
                    "waiting" => StatusKind::Waiting,
                    _ => StatusKind::Info,
                };
                let message = server_msg
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let emoji = status_emoji(&kind);

                let template = DestroyProgressLineFragment { emoji: emoji.to_string(), message: message.to_string() };
                if let Ok(html) = template.render() {
                    let event = Event::default().event("progress").data(html);
                    yield Ok(event);
                }
            }
        }

        // Channel closed — destroy_lab() has finished. Get the result.
        let summary_html = match result_rx.await {
            Ok(Ok(ref response)) => render_summary(response),
            Ok(Err(e)) => {
                let template = DestroySummaryFailedFragment {
                    message: format!("{}", e),
                };
                template.render().unwrap_or_default()
            }
            Err(_) => {
                let template = DestroySummaryFailedFragment {
                    message: "Lost connection to destroy operation".to_string(),
                };
                template.render().unwrap_or_default()
            }
        };

        let event = Event::default().event("complete").data(summary_html);
        yield Ok(event);
    }
}

/// Convert a progress message receiver and a typed result receiver into a JSON SSE
/// event stream for REST API consumption.
///
/// The stream yields:
/// - `event: progress` — JSON status messages as they arrive from the progress channel
/// - `event: complete` — final JSON result when the operation succeeds
/// - `event: error` — JSON error message if the operation fails
pub fn json_progress_stream<T: Serialize + Send + 'static>(
    mut rx: mpsc::UnboundedReceiver<Message>,
    result_rx: tokio::sync::oneshot::Receiver<anyhow::Result<T>>,
) -> impl Stream<Item = Result<Event, Infallible>> {
    async_stream::stream! {
        while let Some(msg) = rx.recv().await {
            if let Message::Text(text) = msg {
                let event = Event::default().event("progress").data(text.to_string());
                yield Ok(event);
            }
        }

        match result_rx.await {
            Ok(Ok(response)) => {
                if let Ok(json) = serde_json::to_string(&response) {
                    let event = Event::default().event("complete").data(json);
                    yield Ok(event);
                }
            }
            Ok(Err(e)) => {
                let error = serde_json::json!({ "error": format!("{:#}", e) });
                let event = Event::default().event("error").data(error.to_string());
                yield Ok(event);
            }
            Err(_) => {
                let error = serde_json::json!({ "error": "Lost connection to operation" });
                let event = Event::default().event("error").data(error.to_string());
                yield Ok(event);
            }
        }
    }
}

/// Convert a progress message receiver and an up result receiver into an SSE
/// event stream suitable for consumption by HTMX's SSE extension.
///
/// The stream yields:
/// - `progress` events for each status message received on the channel
/// - A final `complete` event with a summary after the up operation finishes
pub fn up_progress_stream(
    mut rx: mpsc::UnboundedReceiver<Message>,
    result_rx: tokio::sync::oneshot::Receiver<anyhow::Result<UpResponse>>,
) -> impl Stream<Item = Result<Event, Infallible>> {
    async_stream::stream! {
        // Drain all progress messages from the channel
        while let Some(msg) = rx.recv().await {
            if let Message::Text(text) = msg
                && let Ok(server_msg) = serde_json::from_str::<serde_json::Value>(&text)
            {
                let kind_str = server_msg
                    .get("kind")
                    .and_then(|v| v.as_str())
                    .unwrap_or("info");
                let kind = match kind_str {
                    "progress" => StatusKind::Progress,
                    "done" => StatusKind::Done,
                    "waiting" => StatusKind::Waiting,
                    _ => StatusKind::Info,
                };
                let message = server_msg
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let emoji = status_emoji(&kind);

                let template = DestroyProgressLineFragment { emoji: emoji.to_string(), message: message.to_string() };
                if let Ok(html) = template.render() {
                    let event = Event::default().event("progress").data(html);
                    yield Ok(event);
                }
            }
        }

        // Channel closed — up_lab() has finished. Get the result.
        let summary_html = match result_rx.await {
            Ok(Ok(ref response)) => render_up_summary(response),
            Ok(Err(e)) => {
                let template = CreateSummaryFailedFragment {
                    message: format!("{:#}", e),
                };
                template.render().unwrap_or_default()
            }
            Err(_) => {
                let template = CreateSummaryFailedFragment {
                    message: "Lost connection to create operation".to_string(),
                };
                template.render().unwrap_or_default()
            }
        };

        let event = Event::default().event("complete").data(summary_html);
        yield Ok(event);
    }
}

/// Render the appropriate summary template based on up result.
fn render_up_summary(response: &UpResponse) -> String {
    if response.success {
        let template = CreateSummarySuccessFragment {
            lab_id: response.lab_info.id.clone(),
            lab_name: response.lab_info.name.clone(),
            containers: response.summary.containers_created,
            vms: response.summary.vms_created,
            unikernels: response.summary.unikernels_created,
            networks: response.summary.networks_created,
            bridges: response.summary.bridges_created,
            interfaces: response.summary.interfaces_created,
            total_time_secs: response.total_time_secs,
        };
        template.render().unwrap_or_default()
    } else {
        let errors: Vec<String> = response
            .errors
            .iter()
            .map(|e| format!("[{}] {}", e.phase, e.message))
            .collect();
        let template = CreateSummaryFailedFragment {
            message: errors.join("; "),
        };
        template.render().unwrap_or_default()
    }
}

/// Render the appropriate summary template based on destroy result.
fn render_summary(response: &DestroyResponse) -> String {
    if response.success {
        let template = DestroySummarySuccessFragment {
            lab_name: response.lab_name.clone(),
            containers: response.summary.containers_destroyed.len(),
            vms: response.summary.vms_destroyed.len(),
            disks: response.summary.disks_deleted.len(),
            networks: response.summary.docker_networks_destroyed.len()
                + response.summary.libvirt_networks_destroyed.len(),
            interfaces: response.summary.interfaces_deleted.len(),
        };
        template.render().unwrap_or_default()
    } else {
        let template = DestroySummaryErrorsFragment {
            lab_name: response.lab_name.clone(),
            errors: response.errors.clone(),
        };
        template.render().unwrap_or_default()
    }
}
