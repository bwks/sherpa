//! The node console `WebSocket`s: a node's serial or VNC console relayed to its lab's owner.
//!
//! `GET /api/v1/labs/{id}/nodes/{node_name}/console` (serial) and `.../vnc`, upgraded to a
//! WebSocket. Only the lab's owner may open one, admins included, with the bearer token of
//! their login: a browser's session cookie is not taken, so no web page can open a console from
//! the owner's browser. Everything that can refuse the console is checked before the upgrade,
//! so a refusal is an ordinary API error. Once open, binary frames carry the console's bytes
//! each way, as they are: the serial bytes, or the RFB protocol for VNC; text frames are
//! ignored. One session holds a console at a time; `?force=true` takes it over from the owner's
//! own session.
//!
//! The session ends when the client closes it, when the node's console closes (it stopped),
//! when another session takes it over, and when the login session behind the token ends. The
//! last three are told in the close frame's reason. Each session opened, refused and closed is
//! logged as a `console.*` event with the user, lab, node and console (`serial` or `vnc`), never
//! the bytes.
//!
//! `?check=true` asks the same route whether the console can be opened now, without a WebSocket
//! upgrade and without taking it: the same checks run, a refusal is the same API error, and a
//! console that can be opened is answered `200` with a [`ConsoleCheck`] naming the session that
//! holds it, if any. `sherpa vnc` asks before it opens its local port.

use std::fmt;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use axum::extract::connect_info::ConnectInfo;
use axum::extract::ws::rejection::WebSocketUpgradeRejection;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade, close_code};
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::Deserialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tracing::instrument;

use shared::data::ConsoleCheck;

use crate::api::errors::ApiError;
use crate::api::extractors::BearerUser;
use crate::auth::context::AuthContext;
use crate::auth::session::{self, SessionLookup};
use crate::daemon::console_sessions::{Claimant, ConsoleClaim, ConsoleSessions};
use crate::daemon::state::AppState;
use crate::konst::CONSOLE_SESSION_CHECK_SECS;
use crate::services::console::{self, ConsoleKind, ConsoleTarget};
use crate::services::error::ServiceError;

/// Bytes read from a serial console socket at a time, and so the largest frame sent.
const SERIAL_READ_BUFFER: usize = 4096;
/// Bytes read from a VNC console socket at a time: screen updates come in large writes.
const VNC_READ_BUFFER: usize = 65536;
/// Close code of a session another session took over.
const CLOSE_TAKEN_OVER: u16 = 4001;
/// Close code of a session whose login session ended.
const CLOSE_LOGIN_ENDED: u16 = 4002;
/// The longest reason a close frame can carry, in bytes.
const CLOSE_REASON_MAX: usize = 123;

/// The query of the console route.
#[derive(Debug, Default, Deserialize)]
pub struct ConsoleQuery {
    /// Take the console over from a session of the caller's own that holds it.
    #[serde(default)]
    pub force: bool,
    /// Only check that the console can be opened now: run the route's checks and answer, with
    /// no upgrade, and take no session.
    #[serde(default)]
    pub check: bool,
}

/// A WebSocket upgrade, or why the request is not one: a check needs none.
type Upgrade = Result<WebSocketUpgrade, WebSocketUpgradeRejection>;

/// A request for a node console, from the route's path and query and the connection.
struct ConsoleRequest {
    kind: ConsoleKind,
    lab_id: String,
    node_name: String,
    force: bool,
    client: Option<SocketAddr>,
}

/// A console the caller may open, and the socket its running node was started with.
struct CheckedConsole {
    target: ConsoleTarget,
    path: String,
}

/// A console checked and claimed, with its socket connected, ready to relay.
struct OpenConsole {
    kind: ConsoleKind,
    target: ConsoleTarget,
    ctx: AuthContext,
    client: Option<SocketAddr>,
    socket: UnixStream,
    claim: ConsoleClaim,
}

/// Why a console session ended.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Ended {
    /// The client closed it.
    Client,
    /// The node's console closed: the node stopped.
    Node,
    /// Another session took the console over; it names who.
    TakenOver(String),
    /// The login session behind the token ended.
    LoggedOut,
    /// Relaying failed.
    Failed(String),
}

/// Open a node's serial console as a WebSocket, or with `?check=true` say whether it can be.
///
/// GET `/api/v1/labs/{id}/nodes/{node_name}/console[?force=true|?check=true]`
///
/// # Errors
///
/// 401 without a valid bearer token; 403 for anyone but the lab's owner; 404 for a lab or node
/// that does not exist; 400 for a container; 409 when the node is not running, was started
/// before consoles moved behind sherpad, or another session holds its console; 500 when the
/// console's socket cannot be reached.
pub async fn node_console_handler(
    BearerUser(auth): BearerUser,
    State(state): State<AppState>,
    Path((lab_id, node_name)): Path<(String, String)>,
    Query(query): Query<ConsoleQuery>,
    client: Option<Extension<ConnectInfo<SocketAddr>>>,
    upgrade: Upgrade,
) -> Result<Response, ApiError> {
    let request = ConsoleRequest {
        kind: ConsoleKind::Serial,
        lab_id,
        node_name,
        force: query.force,
        client: client.map(|Extension(ConnectInfo(address))| address),
    };
    serve_console(auth.into_context(), state, request, query.check, upgrade).await
}

/// Open a node's VNC console as a WebSocket, carrying the RFB protocol, or with `?check=true`
/// say whether it can be.
///
/// GET `/api/v1/labs/{id}/nodes/{node_name}/vnc[?force=true|?check=true]`
///
/// # Errors
///
/// As [`node_console_handler`], and 400 for a unikernel, which has no VNC console.
pub async fn node_vnc_handler(
    BearerUser(auth): BearerUser,
    State(state): State<AppState>,
    Path((lab_id, node_name)): Path<(String, String)>,
    Query(query): Query<ConsoleQuery>,
    client: Option<Extension<ConnectInfo<SocketAddr>>>,
    upgrade: Upgrade,
) -> Result<Response, ApiError> {
    let request = ConsoleRequest {
        kind: ConsoleKind::Vnc,
        lab_id,
        node_name,
        force: query.force,
        client: client.map(|Extension(ConnectInfo(address))| address),
    };
    serve_console(auth.into_context(), state, request, query.check, upgrade).await
}

/// Answer the console `request` names for `ctx`: with `check`, whether it can be opened now;
/// otherwise open it and relay it over `upgrade`. Either way a refusal is an API error.
async fn serve_console(
    ctx: AuthContext,
    state: AppState,
    request: ConsoleRequest,
    check: bool,
    upgrade: Upgrade,
) -> Result<Response, ApiError> {
    if check {
        return match check_console(&ctx, &state, &request).await {
            Ok(console) => Ok(Json(check_answer(
                &state.console_sessions,
                &console.target,
                request.kind,
            ))
            .into_response()),
            Err(error) => Err(refused(&ctx, &request, true, error)),
        };
    }
    let upgrade = match upgrade {
        Ok(upgrade) => upgrade,
        Err(rejection) => return Ok(rejection.into_response()),
    };
    match open_console(&ctx, &state, &request).await {
        Ok(console) => Ok(upgrade.on_upgrade(move |websocket| relay(websocket, console, state))),
        Err(error) => Err(refused(&ctx, &request, false, error)),
    }
}

/// The answer to a check of `target`'s `kind` console, which may be opened: who holds it, if
/// anyone. Nothing is claimed.
fn check_answer(
    sessions: &ConsoleSessions,
    target: &ConsoleTarget,
    kind: ConsoleKind,
) -> ConsoleCheck {
    ConsoleCheck {
        held_by: sessions
            .holder(&target.lab_id, &target.node_name, kind)
            .map(|holder| holder.to_string()),
    }
}

/// Log the refusal of the console `request` names to `ctx`, by a check or an open, and turn it
/// into the API's error.
fn refused(
    ctx: &AuthContext,
    request: &ConsoleRequest,
    check: bool,
    error: anyhow::Error,
) -> ApiError {
    tracing::warn!(
        event = "console.refused",
        username = %ctx.username,
        lab_id = %request.lab_id,
        node = %request.node_name,
        console = request.kind.suffix(),
        client = ?request.client,
        force = request.force,
        check,
        reason = %format!("{error:#}"),
        "Console refused"
    );
    ApiError::from(error)
}

/// Check that `ctx` may open the console `request` names and that its node runs with it on a
/// socket: the checks of both opening a console and asking whether it can be.
#[instrument(skip(ctx, state, request), fields(username = %ctx.username, lab_id = %request.lab_id, node = %request.node_name, console = request.kind.suffix()))]
async fn check_console(
    ctx: &AuthContext,
    state: &AppState,
    request: &ConsoleRequest,
) -> Result<CheckedConsole> {
    let target = console::console_target(
        ctx,
        &state.db,
        &request.lab_id,
        &request.node_name,
        request.kind,
    )
    .await?;
    let path = console::console_socket(state, &target, request.kind).await?;
    Ok(CheckedConsole { target, path })
}

/// Check that `ctx` may open the console `request` names and that it can be opened, claim it,
/// and connect to its socket.
#[instrument(skip(ctx, state, request), fields(username = %ctx.username, lab_id = %request.lab_id, node = %request.node_name, console = request.kind.suffix()))]
async fn open_console(
    ctx: &AuthContext,
    state: &AppState,
    request: &ConsoleRequest,
) -> Result<OpenConsole> {
    let ConsoleRequest {
        kind,
        ref lab_id,
        ref node_name,
        force,
        client,
    } = *request;
    let CheckedConsole { target, path } = check_console(ctx, state, request).await?;
    let claimant = Claimant {
        username: ctx.username.clone(),
        client,
    };
    let claim = state
        .console_sessions
        .claim(lab_id, node_name, kind, claimant, force)
        .map_err(|holder| ServiceError::Conflict(in_use(kind, node_name, &holder.to_string())))?;
    let socket = UnixStream::connect(&path)
        .await
        .with_context(|| format!("Cannot connect to the {kind} of {node_name} at {path}"))?;
    Ok(OpenConsole {
        kind,
        target,
        ctx: ctx.clone(),
        client,
        socket,
        claim,
    })
}

/// Why the `kind` console of `node_name` is refused while `holder` holds it.
fn in_use(kind: ConsoleKind, node_name: &str, holder: &str) -> String {
    format!(
        "The {} of {node_name} is in use by {holder}. Close that session, or take it over with \
         'sherpa {} {node_name} --force'.",
        kind.name(),
        kind.command()
    )
}

/// Relay `console` over `websocket` until the session ends, then close it with the reason.
#[instrument(skip_all)]
async fn relay(mut websocket: WebSocket, mut console: OpenConsole, state: AppState) {
    let started = Instant::now();
    tracing::info!(
        event = "console.opened",
        username = %console.ctx.username,
        lab_id = %console.target.lab_id,
        node = %console.target.node_name,
        console = console.kind.suffix(),
        client = ?console.client,
        took_over_from = console.claim.displaced.as_ref().map(ToString::to_string),
        "Console session opened"
    );
    let ended = pump(&mut websocket, &mut console, &state).await;
    if let Some(frame) = ended.close_frame(console.kind)
        && let Err(error) = websocket.send(Message::Close(Some(frame))).await
    {
        tracing::debug!(%error, "Cannot send the console's close frame");
    }
    tracing::info!(
        event = "console.closed",
        username = %console.ctx.username,
        lab_id = %console.target.lab_id,
        node = %console.target.node_name,
        console = console.kind.suffix(),
        client = ?console.client,
        duration_secs = started.elapsed().as_secs(),
        reason = %ended,
        "Console session closed"
    );
}

/// Pass bytes between `websocket` and `console`'s socket until the session ends, and say why.
async fn pump(websocket: &mut WebSocket, console: &mut OpenConsole, state: &AppState) -> Ended {
    let mut buffer = vec![
        0;
        match console.kind {
            ConsoleKind::Serial => SERIAL_READ_BUFFER,
            ConsoleKind::Vnc => VNC_READ_BUFFER,
        }
    ];
    let mut login_check = tokio::time::interval(Duration::from_secs(CONSOLE_SESSION_CHECK_SECS));
    // The first tick is now, and the login was checked when the console opened.
    login_check.tick().await;
    loop {
        tokio::select! {
            () = console.claim.taken_over() => {
                let by = state
                    .console_sessions
                    .holder(&console.target.lab_id, &console.target.node_name, console.kind)
                    .map_or_else(|| "another session".to_owned(), |holder| holder.to_string());
                return Ended::TakenOver(by);
            }
            message = websocket.recv() => match message {
                Some(Ok(Message::Binary(bytes))) => {
                    if let Err(error) = console.socket.write_all(&bytes).await {
                        return Ended::Failed(format!("writing to the console: {error}"));
                    }
                }
                Some(Ok(Message::Close(_))) | None => return Ended::Client,
                Some(Ok(_)) => {}
                Some(Err(error)) => return Ended::Failed(format!("reading the WebSocket: {error}")),
            },
            read = console.socket.read(&mut buffer) => match read {
                Ok(0) => return Ended::Node,
                Ok(count) => {
                    let frame = Message::Binary(buffer[..count].to_vec().into());
                    if let Err(error) = websocket.send(frame).await {
                        return Ended::Failed(format!("writing to the WebSocket: {error}"));
                    }
                }
                Err(error) => return Ended::Failed(format!("reading the console: {error}")),
            },
            _ = login_check.tick() => match session::lookup(&state.db, &console.ctx.session_id).await {
                Ok(SessionLookup::Active(_)) => {}
                Ok(SessionLookup::Rejected(_)) => return Ended::LoggedOut,
                Err(error) => {
                    tracing::warn!(error = %format!("{error:#}"), "Cannot check the login session behind a console");
                }
            },
        }
    }
}

impl Ended {
    /// The close frame that tells the client of a `kind` console why, or `None` when the client
    /// closed it.
    fn close_frame(&self, kind: ConsoleKind) -> Option<CloseFrame> {
        let (code, reason) = match (self, kind) {
            (Self::Client, _) => return None,
            (Self::Node, ConsoleKind::Serial) => (
                close_code::NORMAL,
                "The node's console closed: the node stopped.".to_owned(),
            ),
            // QEMU also closes a VNC client that breaks the protocol, so it is not always a stop.
            (Self::Node, ConsoleKind::Vnc) => (
                close_code::NORMAL,
                "The node's VNC console closed the connection.".to_owned(),
            ),
            (Self::TakenOver(by), _) => (CLOSE_TAKEN_OVER, format!("Taken over by {by}.")),
            (Self::LoggedOut, _) => (
                CLOSE_LOGIN_ENDED,
                "Your login session ended. Log in again with 'sherpa login'.".to_owned(),
            ),
            (Self::Failed(_), _) => (close_code::ERROR, "The console relay failed.".to_owned()),
        };
        let reason = &reason[..reason.floor_char_boundary(CLOSE_REASON_MAX)];
        Some(CloseFrame {
            code,
            reason: reason.into(),
        })
    }
}

impl fmt::Display for Ended {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Client => write!(f, "closed by the client"),
            Self::Node => write!(f, "the node's console closed"),
            Self::TakenOver(by) => write!(f, "taken over by {by}"),
            Self::LoggedOut => write!(f, "the login session ended"),
            Self::Failed(error) => write!(f, "failed {error}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn console_end_007_each_way_the_server_ends_a_session_has_its_own_close_code_and_reason() {
        let taken = Ended::TakenOver("alice from 192.0.2.10 since 2026-11-18 09:08:07 UTC".into());
        for (ended, code, reason) in [
            (
                Ended::Node,
                close_code::NORMAL,
                "The node's console closed: the node stopped.",
            ),
            (
                taken,
                CLOSE_TAKEN_OVER,
                "Taken over by alice from 192.0.2.10 since 2026-11-18 09:08:07 UTC.",
            ),
            (
                Ended::LoggedOut,
                CLOSE_LOGIN_ENDED,
                "Your login session ended. Log in again with 'sherpa login'.",
            ),
            (
                Ended::Failed("reading the console: reset".into()),
                close_code::ERROR,
                "The console relay failed.",
            ),
        ] {
            let frame = ended.close_frame(ConsoleKind::Serial).unwrap();
            assert_eq!(frame.code, code, "{ended}");
            assert_eq!(frame.reason.as_str(), reason, "{ended}");
            if ended != Ended::Node {
                assert_eq!(
                    ended.close_frame(ConsoleKind::Vnc).unwrap(),
                    frame,
                    "{ended}"
                );
            }
        }
        // A VNC console the node closed does not claim the node stopped.
        let frame = Ended::Node.close_frame(ConsoleKind::Vnc).unwrap();
        assert_eq!(frame.code, close_code::NORMAL);
        assert_eq!(
            frame.reason.as_str(),
            "The node's VNC console closed the connection."
        );
    }

    fn target(node_name: &str) -> ConsoleTarget {
        ConsoleTarget {
            lab_id: "abcd1234".to_owned(),
            node_name: node_name.to_owned(),
            domain: format!("{node_name}-abcd1234"),
        }
    }

    #[test]
    fn console_session_008_a_check_answers_who_holds_the_console_and_takes_nothing() {
        let sessions = ConsoleSessions::default();
        let ubu01 = target("ubu01");
        assert_eq!(
            check_answer(&sessions, &ubu01, ConsoleKind::Vnc),
            ConsoleCheck { held_by: None }
        );
        let alice = Claimant {
            username: "alice".to_owned(),
            client: Some(SocketAddr::from(([192, 0, 2, 10], 50000))),
        };
        let claim = sessions
            .claim("abcd1234", "ubu01", ConsoleKind::Vnc, alice.clone(), false)
            .unwrap();
        let held_by = check_answer(&sessions, &ubu01, ConsoleKind::Vnc)
            .held_by
            .unwrap();
        assert!(
            held_by.starts_with("alice from 192.0.2.10 since "),
            "{held_by}"
        );
        // The serial console is held apart, and the check left the VNC one to its holder.
        assert_eq!(
            check_answer(&sessions, &ubu01, ConsoleKind::Serial),
            ConsoleCheck { held_by: None }
        );
        assert_eq!(
            sessions
                .holder("abcd1234", "ubu01", ConsoleKind::Vnc)
                .unwrap()
                .claimant,
            alice
        );
        drop(claim);
        assert_eq!(
            check_answer(&sessions, &ubu01, ConsoleKind::Vnc),
            ConsoleCheck { held_by: None }
        );
    }

    #[test]
    fn console_end_008_a_long_close_reason_is_cut_short_at_a_character_boundary() {
        let frame = Ended::TakenOver("é".repeat(100))
            .close_frame(ConsoleKind::Vnc)
            .unwrap();
        assert!(frame.reason.len() <= CLOSE_REASON_MAX);
        assert!(frame.reason.as_str().starts_with("Taken over by é"));
    }
}
