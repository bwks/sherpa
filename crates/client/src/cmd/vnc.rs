//! `sherpa vnc`: a node's VNC console for a VNC viewer on the user's machine, through the Sherpa
//! server. A listener on `127.0.0.1`, on a port the system picks, takes the viewer's
//! connections; each is relayed to the server's VNC WebSocket, opened with the login's bearer
//! token and the client's TLS settings, until either side closes it. The listener runs until
//! Ctrl+C. The server decides who may open the console and says why it refuses it: the command
//! asks it first, with the route's `?check=true`, and ends with the refusal before it opens the
//! port. Other accounts on the user's machine can reach the loopback listener while it runs, and
//! QEMU's VNC server asks for no password. The server side is "Node consoles" in
//! `docs/architecture/SERVER.md`.

use std::future::Future;
use std::io;
use std::net::{Ipv4Addr, SocketAddr};
use std::pin::pin;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use futures_util::{FutureExt, SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;
use tokio_tungstenite::tungstenite::Message;

use shared::data::{ClientConfig, ConsoleCheck, LabIdentity};

use super::console::{Route, console_url, http_answer, open_error};
use crate::token::load_token;
use crate::ws_client::{WebSocketClient, WsStream};

/// Bytes read from the viewer at a time. A viewer sends little: keys, pointer moves and
/// requests for screen updates.
const VIEWER_READ_BUFFER: usize = 16384;

/// The VNC console of one node, as each viewer connection opens it.
struct VncRoute {
    name: String,
    client: WebSocketClient,
    token: String,
}

/// How a viewer's connection ended.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Ended {
    Viewer,
    Server(Option<String>),
}

/// Relay the VNC console of node `name` in `lab` to VNC viewers on this machine, through the
/// Sherpa server at `server_url`, until Ctrl+C. With `force`, each connection takes the console
/// over from a session of the user's own that holds it.
///
/// # Errors
///
/// Fails when the server refuses the console before the listener is opened (not logged in, not
/// the owner, no such node, a node with no VNC console, not running or still on a VNC port), when
/// the listener cannot be opened, or when the server later refuses a viewer for a reason asking
/// again cannot change.
#[tracing::instrument(skip_all, fields(%name, lab_id = %lab.id, force))]
pub async fn vnc(
    name: &str,
    lab: &LabIdentity,
    force: bool,
    server_url: &str,
    config: &ClientConfig,
) -> Result<()> {
    let timeout = Duration::from_secs(config.server_connection.timeout_secs);
    let url = console_url(server_url, &lab.id, name, Route::Vnc, force)?;
    let mut check_url = console_url(server_url, &lab.id, name, Route::Vnc, false)?;
    check_url.set_query(Some("check=true"));
    let token = load_token()?;
    let held_by = check(
        &WebSocketClient::new(
            check_url.to_string(),
            timeout,
            config.server_connection.clone(),
        ),
        &token,
        name,
    )
    .await?;
    let route = Arc::new(VncRoute {
        name: name.to_owned(),
        client: WebSocketClient::new(url.to_string(), timeout, config.server_connection.clone()),
        token,
    });
    // One Ctrl+C future for the whole command, polled once now so its handler is in place before
    // the address is printed. A new one per loop turn would miss a Ctrl+C that came while another
    // branch ran, such as a viewer's relay ending.
    let mut interrupt = pin!(tokio::signal::ctrl_c());
    if let Some(interrupted) = (&mut interrupt).now_or_never() {
        interrupted.context("Cannot wait for Ctrl+C")?;
        return Ok(());
    }
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .context("Cannot open a local port for the VNC viewer")?;
    let address = listener
        .local_addr()
        .context("Cannot read the local port for the VNC viewer")?;
    println!("{}", listening_line(name, &lab.name, address));
    if let Some(holder) = held_by {
        eprintln!("{}", in_use_line(name, &holder, force));
    }

    serve_until(listener, interrupt, |viewer, peer| {
        serve_viewer(Arc::clone(&route), viewer, peer)
    })
    .await
    .context("Cannot wait for Ctrl+C")?;
    eprintln!("\nStopped relaying the VNC console of {name}.");
    Ok(())
}

/// Ask the server, through `client` at the route's `?check=true`, whether the VNC console of
/// `name` can be opened now, and say who holds it, if anyone. The server answers a check with
/// HTTP instead of the WebSocket upgrade: `200` with a [`ConsoleCheck`], or the refusal opening
/// the console would get.
///
/// # Errors
///
/// The server's refusal, in its words, or the error when it cannot be reached.
async fn check(client: &WebSocketClient, token: &str, name: &str) -> Result<Option<String>> {
    let error = match client
        .open(&[("Authorization", format!("Bearer {token}"))])
        .await
    {
        Ok(mut stream) => {
            // A server that upgraded did not take the check: let the session go, and let each
            // viewer's connection be refused, as before checks.
            let _ = stream.close(None).await;
            return Ok(None);
        }
        Err(error) => error,
    };
    match http_answer(&error) {
        // A body cut off from the answer only loses who holds the console.
        Some((200, body)) => Ok(body
            .and_then(|body| serde_json::from_slice::<ConsoleCheck>(&body).ok())
            .and_then(|check| check.held_by)),
        _ => Err(open_error(name, Route::Vnc, error).into_error()),
    }
}

/// Accept viewers on `listener`, each served by `serve` in a task of its own, until `interrupt`
/// resolves; its error is returned as the error. A viewer whose `serve` fails ends the command
/// with that error: a refusal every new connection would meet again.
async fn serve_until<I, S, F>(listener: TcpListener, interrupt: I, serve: S) -> Result<()>
where
    I: Future<Output = io::Result<()>>,
    S: Fn(TcpStream, SocketAddr) -> F,
    F: Future<Output = Result<()>> + Send + 'static,
{
    let mut interrupt = pin!(interrupt);
    let mut viewers = JoinSet::new();
    loop {
        tokio::select! {
            interrupted = &mut interrupt => return interrupted.map_err(Into::into),
            accepted = listener.accept() => match accepted {
                Ok((viewer, peer)) => {
                    viewers.spawn(serve(viewer, peer));
                }
                Err(error) => eprintln!("Cannot accept a VNC viewer: {error}"),
            },
            Some(served) = viewers.join_next() => match served {
                Ok(Ok(())) => {}
                Ok(Err(refused)) => return Err(refused),
                Err(error) => eprintln!("A VNC viewer's relay failed: {error}"),
            },
        }
    }
}

/// Relay `viewer`, connected from `peer`, to the node's VNC console until either side closes.
/// A refusal the next connection would meet again is returned; any other failure is told and
/// ends only this connection.
async fn serve_viewer(route: Arc<VncRoute>, viewer: TcpStream, peer: SocketAddr) -> Result<()> {
    let name = &route.name;
    let stream = match route
        .client
        .open(&[("Authorization", format!("Bearer {}", route.token))])
        .await
    {
        Ok(stream) => stream,
        Err(error) => {
            let error = open_error(name, Route::Vnc, error);
            if error.lasting() {
                return Err(error.into_error());
            }
            eprintln!("Viewer {peer}: {:#}", error.into_error());
            return Ok(());
        }
    };
    eprintln!("Viewer {peer} connected to the VNC console of {name}.");
    let ended = relay(viewer, stream).await;
    eprintln!("{}", closed_line(name, peer, &ended));
    Ok(())
}

/// Pass bytes between `viewer` and the VNC WebSocket `stream` until either closes, and say who
/// did. A failure on either side ends it as that side closing.
async fn relay(viewer: TcpStream, stream: WsStream) -> Ended {
    let (mut viewer_read, mut viewer_write) = viewer.into_split();
    let (mut sink, mut stream) = stream.split();
    let mut buffer = vec![0; VIEWER_READ_BUFFER];
    let ended = loop {
        tokio::select! {
            read = viewer_read.read(&mut buffer) => match read {
                Ok(0) | Err(_) => break Ended::Viewer,
                Ok(count) => {
                    let frame = Message::Binary(buffer[..count].to_vec().into());
                    if let Err(error) = sink.send(frame).await {
                        break Ended::Server(Some(error.to_string()));
                    }
                }
            },
            message = stream.next() => match message {
                Some(Ok(Message::Binary(bytes))) => {
                    if viewer_write.write_all(&bytes).await.is_err() {
                        break Ended::Viewer;
                    }
                }
                Some(Ok(Message::Close(frame))) => {
                    break Ended::Server(
                        frame
                            .map(|frame| frame.reason.to_string())
                            .filter(|reason| !reason.is_empty()),
                    );
                }
                Some(Ok(_)) => {}
                Some(Err(error)) => break Ended::Server(Some(error.to_string())),
                None => break Ended::Server(None),
            },
        }
    };
    // Either side may be gone already; nothing is left to tell it.
    let _ = sink.close().await;
    let _ = viewer_write.shutdown().await;
    ended
}

/// The line on stdout once the listener is open: where to point the viewer, and how to stop.
fn listening_line(name: &str, lab_name: &str, address: SocketAddr) -> String {
    format!(
        "VNC console of {name} in lab {lab_name}: connect a VNC viewer to {address}. Ctrl+C to \
         stop."
    )
}

/// The line on stderr when the console is held by `holder` as the listener opens: viewers are
/// refused until it lets go, or with `force` take it over.
fn in_use_line(name: &str, holder: &str, force: bool) -> String {
    if force {
        format!(
            "The VNC console of {name} is in use by {holder}: a viewer connecting takes it over."
        )
    } else {
        format!(
            "The VNC console of {name} is in use by {holder}: viewers are refused until that \
             session ends, or take it over with 'sherpa vnc {name} --force'."
        )
    }
}

/// The line on stderr when the connection of the viewer at `peer` ends.
fn closed_line(name: &str, peer: SocketAddr, ended: &Ended) -> String {
    match ended {
        Ended::Viewer => format!("Viewer {peer} disconnected from the VNC console of {name}."),
        Ended::Server(Some(reason)) => {
            format!("The VNC console of {name} closed for viewer {peer}: {reason}")
        }
        Ended::Server(None) => format!("The VNC console of {name} closed for viewer {peer}."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::exit_status;
    use crate::error::ClientError;
    use std::future::pending;
    use std::sync::Mutex;

    use shared::data::ServerConnection;
    use tokio::sync::oneshot;
    use tokio::task::JoinHandle;

    /// The exit status of a failed command, as a refused viewer's ended it before checks.
    const FAILED: u8 = 1;
    use tokio_tungstenite::tungstenite::protocol::CloseFrame;
    use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
    use tokio_tungstenite::{WebSocketStream, accept_async, connect_async};

    #[test]
    fn vnc_console_008_with_force_a_viewer_connecting_takes_the_console_over() {
        let holder = "dev from 192.0.2.10 since 2026-11-18 09:08:07 UTC";
        assert_eq!(
            in_use_line("ubu01", holder, true),
            "The VNC console of ubu01 is in use by dev from 192.0.2.10 since 2026-11-18 09:08:07 \
             UTC: a viewer connecting takes it over."
        );
    }

    /// A server that answers one request with HTTP `status` and the JSON `body`, as sherpad
    /// answers a check, and the request it got.
    async fn answering(status: &str, body: &str) -> (WebSocketClient, JoinHandle<String>) {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let url = format!(
            "ws://{}/api/v1/labs/abcd1234/nodes/ubu01/vnc?check=true",
            listener.local_addr().unwrap()
        );
        let response = format!(
            "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n\
             {body}",
            body.len()
        );
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 1024];
            while !request.ends_with(b"\r\n\r\n") {
                let count = socket.read(&mut buffer).await.unwrap();
                assert!(count > 0, "the request ended early");
                request.extend_from_slice(&buffer[..count]);
            }
            socket.write_all(response.as_bytes()).await.unwrap();
            String::from_utf8(request).unwrap()
        });
        let client = WebSocketClient::new(url, Duration::from_secs(5), ServerConnection::default());
        (client, server)
    }

    fn api_error(code: &str, message: &str, details: Option<&str>) -> String {
        serde_json::json!({
            "error": { "code": code, "message": message, "details": details }
        })
        .to_string()
    }

    #[tokio::test]
    async fn vnc_console_003_a_refusal_as_the_command_starts_ends_it_with_the_reason() {
        for (status, body, servers_words) in [
            (
                "403 Forbidden",
                api_error(
                    "FORBIDDEN",
                    "Access denied: only the owner of lab 'abcd1234' can open its consoles",
                    None,
                ),
                Some("Access denied: only the owner of lab 'abcd1234' can open its consoles"),
            ),
            (
                "400 Bad Request",
                api_error(
                    "BAD_REQUEST",
                    "Invalid request",
                    Some(
                        "ctr01 is a container, and containers have no VNC console. Connect \
                         with 'sherpa ssh ctr01' instead.",
                    ),
                ),
                Some(
                    "ctr01 is a container, and containers have no VNC console. Connect with \
                     'sherpa ssh ctr01' instead.",
                ),
            ),
            (
                "409 Conflict",
                api_error(
                    "CONFLICT",
                    "Node 'ubu01' is not running. Start it with 'sherpa resume'.",
                    None,
                ),
                Some("Node 'ubu01' is not running. Start it with 'sherpa resume'."),
            ),
            (
                "409 Conflict",
                api_error(
                    "CONFLICT",
                    "Node 'ubu01' was started before its VNC console moved behind Sherpa. Stop \
                     and resume the lab ('sherpa down', then 'sherpa resume'), or redeploy the \
                     node, to use its VNC console.",
                    None,
                ),
                Some(
                    "Node 'ubu01' was started before its VNC console moved behind Sherpa. Stop \
                     and resume the lab ('sherpa down', then 'sherpa resume'), or redeploy the \
                     node, to use its VNC console.",
                ),
            ),
            (
                "401 Unauthorized",
                api_error("UNAUTHORIZED", "Missing or invalid bearer token", None),
                None,
            ),
        ] {
            let (client, server) = answering(status, &body).await;
            let error = check(&client, "the-token", "ubu01").await.unwrap_err();
            match (error.downcast_ref::<ClientError>(), servers_words) {
                (Some(ClientError::ConsoleRefused { message }), Some(words)) => {
                    assert_eq!(message, words, "{status}");
                }
                (Some(ClientError::ConsoleLoginRefused), None) => {}
                _ => panic!("{status}: not the refusal: {error:#}"),
            }
            assert_eq!(exit_status(&error), FAILED, "{status}");
            let request = server.await.unwrap();
            assert!(
                request.starts_with("GET /api/v1/labs/abcd1234/nodes/ubu01/vnc?check=true "),
                "{request}"
            );
            assert!(
                request
                    .to_ascii_lowercase()
                    .contains("authorization: bearer the-token\r\n"),
                "{request}"
            );
        }
    }

    #[tokio::test]
    async fn vnc_console_004_a_server_that_cannot_be_reached_as_the_command_starts_ends_it() {
        // A port nothing listens on: bound, then closed.
        let address = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap()
            .local_addr()
            .unwrap();
        let client = WebSocketClient::new(
            format!("ws://{address}/api/v1/labs/abcd1234/nodes/ubu01/vnc?check=true"),
            Duration::from_secs(5),
            ServerConnection::default(),
        );
        let error = check(&client, "t", "ubu01").await.unwrap_err();
        assert!(matches!(
            error.downcast_ref::<ClientError>(),
            Some(ClientError::ConsoleUnreachable { route: Route::Vnc, node }) if node == "ubu01"
        ));
        assert_eq!(exit_status(&error), FAILED);
    }

    #[test]
    fn vnc_console_013_the_end_of_each_viewers_connection_is_told_with_its_address() {
        let peer = SocketAddr::from(([127, 0, 0, 1], 50000));
        assert_eq!(
            closed_line("ubu01", peer, &Ended::Viewer),
            "Viewer 127.0.0.1:50000 disconnected from the VNC console of ubu01."
        );
        assert_eq!(
            closed_line(
                "ubu01",
                peer,
                &Ended::Server(Some("The node's VNC console closed the connection.".into()))
            ),
            "The VNC console of ubu01 closed for viewer 127.0.0.1:50000: The node's VNC \
             console closed the connection."
        );
        assert_eq!(
            closed_line("ubu01", peer, &Ended::Server(None)),
            "The VNC console of ubu01 closed for viewer 127.0.0.1:50000."
        );
    }

    /// A viewer connected to a local port, and the WebSocket the relay gets, with the far end of
    /// each for the test to drive: the viewer's socket, and the server's end of the WebSocket.
    async fn connected() -> (TcpStream, TcpStream, WsStream, WebSocketStream<TcpStream>) {
        let viewers = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let viewer = TcpStream::connect(viewers.local_addr().unwrap())
            .await
            .unwrap();
        let (relayed_viewer, _) = viewers.accept().await.unwrap();
        let servers = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let url = format!("ws://{}/", servers.local_addr().unwrap());
        let (client, server) = tokio::join!(connect_async(url), async {
            accept_async(servers.accept().await.unwrap().0)
                .await
                .unwrap()
        });
        (viewer, relayed_viewer, client.unwrap().0, server)
    }

    #[tokio::test]
    async fn vnc_console_010_a_ctrl_c_as_a_viewers_connection_ends_still_stops_the_command() {
        // The Ctrl+C that came as a viewer's relay ended was lost when the loop made a new
        // Ctrl+C future each turn; it must end the command every time.
        for _ in 0..50 {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
            let address = listener.local_addr().unwrap();
            let (interrupt, interrupted) = oneshot::channel::<()>();
            let interrupt = Arc::new(Mutex::new(Some(interrupt)));
            let serve = {
                let interrupt = Arc::clone(&interrupt);
                move |_viewer: TcpStream, _peer: SocketAddr| {
                    // Ctrl+C as the viewer's relay ends.
                    let sender = interrupt.lock().unwrap().take();
                    async move {
                        if let Some(sender) = sender {
                            sender.send(()).unwrap();
                        }
                        Ok(())
                    }
                }
            };
            let command = tokio::spawn(serve_until(
                listener,
                async move { interrupted.await.map_err(io::Error::other) },
                serve,
            ));
            let _viewer = TcpStream::connect(address).await.unwrap();
            tokio::time::timeout(Duration::from_secs(5), command)
                .await
                .expect("the command ended on the interrupt")
                .unwrap()
                .unwrap();
        }
    }

    #[tokio::test]
    async fn vnc_console_009_a_viewer_refused_for_a_lasting_reason_ends_the_command() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let command = tokio::spawn(serve_until(listener, pending(), |_viewer, _peer| async {
            Err(ClientError::ConsoleRefused {
                message: "Access denied".to_owned(),
            }
            .into())
        }));
        let _viewer = TcpStream::connect(address).await.unwrap();
        let error = tokio::time::timeout(Duration::from_secs(5), command)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        assert!(matches!(
            error.downcast_ref::<ClientError>(),
            Some(ClientError::ConsoleRefused { message }) if message == "Access denied"
        ));
    }

    #[tokio::test]
    async fn vnc_console_011_what_the_viewer_sends_reaches_the_node() {
        let (mut viewer, relayed_viewer, stream, mut server) = connected().await;
        let relay = tokio::spawn(relay(relayed_viewer, stream));
        // The server speaks first in RFB.
        server
            .send(Message::Binary(b"RFB 003.008\n".to_vec().into()))
            .await
            .unwrap();
        let mut banner = [0; 12];
        viewer.read_exact(&mut banner).await.unwrap();
        assert_eq!(&banner, b"RFB 003.008\n");
        viewer.write_all(b"RFB 003.008\n").await.unwrap();
        match server.next().await.unwrap().unwrap() {
            Message::Binary(bytes) => assert_eq!(&bytes[..], b"RFB 003.008\n"),
            other => panic!("unexpected frame: {other:?}"),
        }
        drop(viewer);
        assert_eq!(relay.await.unwrap(), Ended::Viewer);
        // The WebSocket is closed when the viewer goes.
        assert!(matches!(
            server.next().await,
            Some(Ok(Message::Close(_))) | None
        ));
    }

    #[tokio::test]
    async fn vnc_console_012_when_the_server_closes_a_session_the_viewers_connection_is_closed() {
        let (mut viewer, relayed_viewer, stream, mut server) = connected().await;
        let relay = tokio::spawn(relay(relayed_viewer, stream));
        server
            .close(Some(CloseFrame {
                code: CloseCode::Library(4001),
                reason: "Taken over by alice.".into(),
            }))
            .await
            .unwrap();
        assert_eq!(
            relay.await.unwrap(),
            Ended::Server(Some("Taken over by alice.".to_owned()))
        );
        let mut rest = Vec::new();
        viewer.read_to_end(&mut rest).await.unwrap();
        assert_eq!(rest, b"");
    }
}
