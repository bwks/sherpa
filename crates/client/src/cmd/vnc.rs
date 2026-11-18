//! `sherpa vnc`: a node's VNC console for a VNC viewer on the user's machine, through the Sherpa
//! server. A listener on `127.0.0.1`, on a port the system picks, takes the viewer's
//! connections; each is relayed to the server's VNC WebSocket, opened with the login's bearer
//! token and the client's TLS settings, until either side closes it. The listener runs until
//! Ctrl+C. The server decides who may open the console and says why it refuses it; see
//! `docs/architecture/CONSOLE_CLIENT.md`.

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

use shared::data::{ClientConfig, LabIdentity};

use super::console::{Route, console_url, open_error};
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
    /// The viewer closed it.
    Viewer,
    /// The server closed it, with its reason when it gave one.
    Server(Option<String>),
}

/// Relay the VNC console of node `name` in `lab` to VNC viewers on this machine, through the
/// Sherpa server at `server_url`, until Ctrl+C. With `force`, each connection takes the console
/// over from a session of the user's own that holds it.
///
/// # Errors
///
/// Fails when the listener cannot be opened, or the server refuses the console for a reason
/// asking again cannot change: not logged in, not the owner, no such node, or a node with no
/// VNC console.
#[tracing::instrument(skip_all, fields(%name, lab_id = %lab.id, force))]
pub async fn vnc(
    name: &str,
    lab: &LabIdentity,
    force: bool,
    server_url: &str,
    config: &ClientConfig,
) -> Result<()> {
    let url = console_url(server_url, &lab.id, name, Route::Vnc, force)?;
    let route = Arc::new(VncRoute {
        name: name.to_owned(),
        client: WebSocketClient::new(
            url.to_string(),
            Duration::from_secs(config.server_connection.timeout_secs),
            config.server_connection.clone(),
        ),
        token: load_token()?,
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

    serve_until(listener, interrupt, |viewer, peer| {
        serve_viewer(Arc::clone(&route), viewer, peer)
    })
    .await
    .context("Cannot wait for Ctrl+C")?;
    eprintln!("\nStopped relaying the VNC console of {name}.");
    Ok(())
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
    use std::future::pending;
    use std::sync::Mutex;

    use anyhow::anyhow;
    use tokio::sync::oneshot;
    use tokio_tungstenite::tungstenite::protocol::CloseFrame;
    use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
    use tokio_tungstenite::{WebSocketStream, accept_async, connect_async};

    #[test]
    fn the_listening_line_gives_the_address_for_the_viewer() {
        let address = SocketAddr::from(([127, 0, 0, 1], 40123));
        assert_eq!(
            listening_line("ubu01", "test", address),
            "VNC console of ubu01 in lab test: connect a VNC viewer to 127.0.0.1:40123. Ctrl+C \
             to stop."
        );
    }

    #[test]
    fn the_close_line_says_who_closed_and_why() {
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
    async fn an_interrupt_as_a_viewer_ends_stops_the_command() {
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
    async fn a_lasting_refusal_ends_the_command_with_it() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let command = tokio::spawn(serve_until(listener, pending(), |_viewer, _peer| async {
            Err(anyhow!("Access denied"))
        }));
        let _viewer = TcpStream::connect(address).await.unwrap();
        let error = tokio::time::timeout(Duration::from_secs(5), command)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        assert_eq!(error.to_string(), "Access denied");
    }

    #[tokio::test]
    async fn bytes_pass_both_ways_until_the_viewer_closes() {
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
    async fn the_servers_close_reason_ends_the_viewers_connection() {
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
