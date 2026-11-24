//! `sherpa console`: a node's serial console in the local terminal, through the Sherpa server.
//! The server relays the node's console socket over a WebSocket on its API, opened with the
//! login's bearer token and the client's TLS settings, and Jato's pass-through runs it in the
//! terminal. There is no SSH hop and no telnet. The server decides who may open a console and
//! says why it refuses one; see "Node consoles" in `docs/architecture/SERVER.md`.

use std::fmt;
use std::io;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use futures_util::{SinkExt, StreamExt};
use jato::{InteractiveOptions, Transport, pass_through};
use serde::Deserialize;
use tokio_tungstenite::tungstenite::{self, Message};
use url::Url;

use shared::data::{ClientConfig, LabIdentity};
use shared::util::term_msg_surround;

use crate::token::load_token;
use crate::ws_client::{WebSocketClient, WsStream};

/// A node console over the server's console WebSocket, as Jato's pass-through drives it.
struct ConsoleSocket {
    stream: WsStream,
    close_reason: Option<String>,
}

/// A node console the server relays: its route, and what the user is told it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Route {
    Serial,
    Vnc,
}

/// The JSON body of an API error.
#[derive(Debug, Deserialize)]
struct ErrorBody {
    error: ErrorDetail,
}

#[derive(Debug, Deserialize)]
struct ErrorDetail {
    message: String,
    #[serde(default)]
    details: Option<String>,
}

/// Open the serial console of the node `name` of `lab` in this process's terminal,
/// through the Sherpa server at `server_url`, until the console closes or an escape (`Ctrl+]`,
/// or `~.` at the start of a line) is typed. With `force`, a session of the user's own that
/// holds the console is taken over.
#[tracing::instrument(skip_all, fields(%name, lab_id = %lab.id, force))]
pub async fn console(
    name: &str,
    lab: &LabIdentity,
    force: bool,
    server_url: &str,
    config: &ClientConfig,
) -> Result<()> {
    let url = console_url(server_url, &lab.id, name, Route::Serial, force)?;
    let token = load_token()?;
    term_msg_surround(&format!("Connecting to: {name}"));
    let client = WebSocketClient::new(
        url.to_string(),
        Duration::from_secs(config.server_connection.timeout_secs),
        config.server_connection.clone(),
    );
    let stream = client
        .open(&[("Authorization", format!("Bearer {token}"))])
        .await
        .map_err(|error| open_error(name, Route::Serial, error).into_error())?;
    let mut socket = ConsoleSocket {
        stream,
        close_reason: None,
    };

    eprintln!("{}", connected_line(name, &lab.name));
    // Sherpa prints its own lines, with the node's name, in place of Jato's.
    let mut interactive = InteractiveOptions::default();
    interactive.banner = false;
    let end = pass_through(&mut socket, name, &interactive)
        .await
        .with_context(|| format!("The console of {name} failed"))?;
    eprintln!(
        "{}",
        closed_line(name, end.escaped, socket.close_reason.as_deref())
    );
    Ok(())
}

/// Why a console could not be opened.
#[derive(Debug)]
pub(super) enum OpenError {
    Refused { status: u16, message: String },
    Failed(anyhow::Error),
}

/// The URL of the `route` console of node `name` in lab `lab_id`, on the server whose RPC
/// WebSocket is at `server_url`: the same scheme, host and port.
pub(super) fn console_url(
    server_url: &str,
    lab_id: &str,
    name: &str,
    route: Route,
    force: bool,
) -> Result<Url> {
    let mut url =
        Url::parse(server_url).with_context(|| format!("Invalid server URL: {server_url}"))?;
    url.path_segments_mut()
        .map_err(|()| anyhow!("Invalid server URL: {server_url}"))?
        .clear()
        .extend(["api", "v1", "labs", lab_id, "nodes", name, route.segment()]);
    url.set_query(force.then_some("force=true"));
    Ok(url)
}

/// What the user is told when the `route` console of `name` cannot be opened: the server's own
/// words when it refused it, and otherwise the error with its cause.
pub(super) fn open_error(name: &str, route: Route, error: anyhow::Error) -> OpenError {
    match http_answer(&error) {
        Some((status, body)) => OpenError::Refused {
            status,
            message: refusal_message(name, route, status, body.as_deref()),
        },
        None => OpenError::Failed(error.context(format!(
            "Failed to open the {route} of {name} through the Sherpa server"
        ))),
    }
}

/// The HTTP status and body the server answered a WebSocket handshake with, when it answered
/// with something other than the upgrade: a refusal, or the answer to a check.
pub(super) fn http_answer(error: &anyhow::Error) -> Option<(u16, Option<Vec<u8>>)> {
    error.chain().find_map(|cause| match cause.downcast_ref() {
        Some(tungstenite::Error::Http(response)) => {
            Some((response.status().as_u16(), response.body().clone()))
        }
        _ => None,
    })
}

/// The message for a `route` console the server refused with HTTP `status` and `body`.
fn refusal_message(name: &str, route: Route, status: u16, body: Option<&[u8]>) -> String {
    let message = body
        .and_then(|body| serde_json::from_slice::<ErrorBody>(body).ok())
        .map(|body| body.error.details.unwrap_or(body.error.message));
    match (status, message) {
        (401, _) => "Not logged in to the Sherpa server, or the login has expired: run \
                     'sherpa login'."
            .to_owned(),
        (_, Some(message)) => message,
        // Only a server from before the console moved behind it has no such route.
        (404, None) => format!(
            "The Sherpa server has no {route} route: it is older than this client. Upgrade the \
             server."
        ),
        (status, None) => {
            format!("The Sherpa server refused the {route} of {name} (HTTP {status}).")
        }
    }
}

impl Route {
    /// The last segment of the console's URL.
    fn segment(self) -> &'static str {
        match self {
            Self::Serial => "console",
            Self::Vnc => "vnc",
        }
    }
}

impl fmt::Display for Route {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Serial => write!(f, "console"),
            Self::Vnc => write!(f, "VNC console"),
        }
    }
}

impl OpenError {
    /// The error the user is shown.
    pub(super) fn into_error(self) -> anyhow::Error {
        match self {
            Self::Refused { message, .. } => anyhow!(message),
            Self::Failed(error) => error,
        }
    }

    /// Whether asking again cannot succeed until something else changes: the login, the
    /// ownership, the node or the server. A console that is held, or a node not running, can be
    /// asked for again.
    pub(super) fn lasting(&self) -> bool {
        match self {
            Self::Refused { status, .. } => *status != 409,
            Self::Failed(_) => false,
        }
    }
}

/// The line on stderr once the console is open, naming the node, its lab and both escapes.
fn connected_line(name: &str, lab_name: &str) -> String {
    format!(
        "Connected to the console of {name} in lab {lab_name}. Escape: Ctrl+] or ~. at the \
         start of a line."
    )
}

/// The line on stderr when the console session ends, with the server's `reason` when it ended
/// it. After an escape, or when the server ended it, the cursor is usually on the node's
/// prompt, so the line starts on a new line.
fn closed_line(name: &str, escaped: bool, reason: Option<&str>) -> String {
    let gap = if escaped || reason.is_some() {
        "\n"
    } else {
        ""
    };
    match reason {
        Some(reason) => format!("{gap}Connection to the console of {name} closed: {reason}"),
        None => format!("{gap}Connection to the console of {name} closed."),
    }
}

impl Transport for ConsoleSocket {
    /// Cancel safe: the one await is the stream's `next`, which keeps a frame it has begun to
    /// read for the next call, and a frame that is not binary is passed over within the call.
    async fn receive(&mut self) -> io::Result<Option<Vec<u8>>> {
        while let Some(message) = self.stream.next().await {
            match message.map_err(io::Error::other)? {
                Message::Binary(bytes) => return Ok(Some(bytes.to_vec())),
                Message::Close(frame) => {
                    self.close_reason = frame
                        .map(|frame| frame.reason.to_string())
                        .filter(|reason| !reason.is_empty());
                    return Ok(None);
                }
                Message::Text(_) | Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {}
            }
        }
        Ok(None)
    }

    async fn send(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.stream
            .send(Message::Binary(bytes.to_vec().into()))
            .await
            .map_err(io::Error::other)
    }

    async fn close(&mut self) -> io::Result<()> {
        self.stream.close(None).await.map_err(io::Error::other)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_tungstenite::tungstenite::http::Response;

    #[test]
    fn serial_console_006_each_console_is_asked_for_at_the_server_the_other_commands_use() {
        assert_eq!(
            console_url(
                "wss://192.0.2.1:3030/ws",
                "abcd1234",
                "alp01",
                Route::Serial,
                false
            )
            .unwrap()
            .as_str(),
            "wss://192.0.2.1:3030/api/v1/labs/abcd1234/nodes/alp01/console"
        );
        assert_eq!(
            console_url(
                "ws://[2001:db8::1]:3030/ws",
                "abcd1234",
                "alp01",
                Route::Serial,
                true
            )
            .unwrap()
            .as_str(),
            "ws://[2001:db8::1]:3030/api/v1/labs/abcd1234/nodes/alp01/console?force=true"
        );
        assert_eq!(
            console_url(
                "wss://192.0.2.1:3030/ws",
                "abcd1234",
                "ubu01",
                Route::Vnc,
                true
            )
            .unwrap()
            .as_str(),
            "wss://192.0.2.1:3030/api/v1/labs/abcd1234/nodes/ubu01/vnc?force=true"
        );
        // A node name is one path segment.
        assert_eq!(
            console_url(
                "wss://192.0.2.1:3030/ws",
                "abcd1234",
                "a/b c",
                Route::Serial,
                false
            )
            .unwrap()
            .path(),
            "/api/v1/labs/abcd1234/nodes/a%2Fb%20c/console"
        );
    }

    #[test]
    fn serial_console_007_a_console_that_cannot_be_reached_fails_naming_it_with_the_cause() {
        for (route, node, says) in [
            (
                Route::Serial,
                "alp01",
                "Failed to open the console of alp01 through the Sherpa server",
            ),
            (
                Route::Vnc,
                "ubu01",
                "Failed to open the VNC console of ubu01 through the Sherpa server",
            ),
        ] {
            let error = open_error(node, route, anyhow!("Connection timeout")).into_error();
            assert_eq!(error.to_string(), says);
            assert_eq!(error.root_cause().to_string(), "Connection timeout");
        }
    }

    fn body(message: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "error": { "code": "CONFLICT", "message": message }
        }))
        .unwrap()
    }

    /// The server's words, from the message and from a bad request's details.
    fn in_the_servers_words() {
        for message in [
            "The console of alp01 is in use by alice from 192.0.2.10 since 2026-11-18 09:08:07 \
             UTC. Close that session, or take it over with 'sherpa console alp01 --force'.",
            "The VNC console of ubu01 is in use by alice from 192.0.2.10 since 2026-11-18 \
             09:08:07 UTC. Close that session, or take it over with 'sherpa vnc ubu01 --force'.",
            "Access denied: only the owner of lab 'abcd1234' can open its consoles",
            "srl01 is a container, and containers have no serial console. Connect with 'sherpa \
             ssh srl01' instead.",
            "Node 'alp01' was started before consoles moved behind Sherpa. Stop and resume the \
             lab ('sherpa down', then 'sherpa resume'), or redeploy the node, to use its \
             console.",
            "Node 'ubu01' was started before its VNC console moved behind Sherpa. Stop and \
             resume the lab ('sherpa down', then 'sherpa resume'), or redeploy the node, to use \
             its VNC console.",
        ] {
            assert_eq!(
                refusal_message("alp01", Route::Serial, 409, Some(&body(message))),
                message
            );
        }
        let details = serde_json::to_vec(&serde_json::json!({
            "error": {
                "code": "BAD_REQUEST",
                "message": "Invalid request",
                "details": "srl01 is a container, and containers have no serial console. Connect \
                            with 'sherpa ssh srl01' instead."
            }
        }))
        .unwrap();
        assert_eq!(
            refusal_message("srl01", Route::Serial, 400, Some(&details)),
            "srl01 is a container, and containers have no serial console. Connect with 'sherpa \
             ssh srl01' instead."
        );
        // A refused WebSocket handshake is found under the connect context.
        let response = Response::builder()
            .status(409)
            .body(Some(body(
                "Node 'alp01' is not running. Start it with 'sherpa resume'.",
            )))
            .unwrap();
        let error = anyhow::Error::new(tungstenite::Error::Http(Box::new(response)))
            .context("Failed to connect to secure WebSocket");
        assert_eq!(
            open_error("alp01", Route::Serial, error)
                .into_error()
                .to_string(),
            "Node 'alp01' is not running. Start it with 'sherpa resume'."
        );
    }

    /// A 401 as not logged in, and an answer with no words of the server's by its status.
    fn without_the_servers_words() {
        assert_eq!(
            refusal_message(
                "alp01",
                Route::Serial,
                401,
                Some(&body("Missing or invalid bearer token"))
            ),
            "Not logged in to the Sherpa server, or the login has expired: run 'sherpa login'."
        );
        assert_eq!(
            refusal_message(
                "alp01",
                Route::Serial,
                502,
                Some(b"<html>bad gateway</html>")
            ),
            "The Sherpa server refused the console of alp01 (HTTP 502)."
        );
        assert_eq!(
            refusal_message("ubu01", Route::Vnc, 502, None),
            "The Sherpa server refused the VNC console of ubu01 (HTTP 502)."
        );
    }

    #[test]
    fn console_refusal_008_the_cli_tells_a_refusal_in_the_servers_words_or_by_its_status() {
        in_the_servers_words();
        without_the_servers_words();
    }

    #[test]
    fn console_refusal_009_a_server_with_no_console_route_is_told_as_older_than_the_client() {
        assert_eq!(
            refusal_message("alp01", Route::Serial, 404, None),
            "The Sherpa server has no console route: it is older than this client. Upgrade \
             the server."
        );
        assert_eq!(
            refusal_message("ubu01", Route::Vnc, 404, None),
            "The Sherpa server has no VNC console route: it is older than this client. Upgrade \
             the server."
        );
    }
}
