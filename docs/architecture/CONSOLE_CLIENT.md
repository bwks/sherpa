# Console client

How `sherpa console` opens a lab node's serial console, and how `sherpa vnc` relays a VM's VNC
console to a VNC viewer ([below](#sherpa-vnc)). The console command is in
`crates/client/src/cmd/console.rs`. It connects to sherpad's console WebSocket, which relays
the node's console socket (see [`SERVER.md`](SERVER.md#node-consoles)); the terminal
pass-through is the Jato library (`bwks/jato-lib`, 0.6.0) with its `terminal` feature. There is
no SSH hop and no telnet: the client needs its Sherpa login, not an SSH key or an OS account on
the server.

## Connecting

`cmd::console::console`:

1. `cli::run_remote` takes the lab's identity from the folder (see
   [`LAB_FOLDER.md`](LAB_FOLDER.md#lab-identity)): from `.sherpa/lab-info.toml` when there is
   one, so the command works in a folder made by `sherpa download`, with no manifest.
2. `console_url` makes the route from the server URL the other commands use
   (`wss://<server>:<port>/ws`): the same scheme, host and port, path
   `/api/v1/labs/<lab-id>/nodes/<node>/console` (`Route::Serial`; `.../vnc` for `Route::Vnc`),
   each part one path segment, and `?force=true` with `--force`.
3. `WebSocketClient::open` opens it with `Authorization: Bearer <token>`, the login token
   `load_token` reads. It is the connect path `WebSocketClient::connect` uses for the RPC
   socket, so TLS is the same: the custom CA, the certificate trusted on first use and
   `insecure` from the client config, and the connection timeout.
4. `jato::pass_through` runs the terminal over `ConsoleSocket`, the WebSocket as a
   `jato::Transport`, until an escape is typed or the server closes the session.

The client does not check who owns the lab, or whether the node is a container or running:
the server answers the upgrade with an API error before anything connects, and the client
prints the server's message (`refusal_message`).

## The transport

`ConsoleSocket` implements `jato::Transport`:

- `receive` returns the next binary frame's bytes. A close frame ends the session, and its
  reason is kept for the close line. Text, ping and pong frames are passed over. It is cancel
  safe, as Jato requires (Jato drops a pending `receive` when a key is typed): its one await is
  the stream's `next`, which keeps a frame it has begun to read for the next call.
- `send` sends the typed bytes as one binary frame.
- `close` sends a close frame. Jato calls it after an escape or a failure.

## Lines and escapes

Jato's banner is off (`InteractiveOptions::banner = false`). Sherpa prints its own lines on
stderr, with the node's name:

- after connecting: `Connected to the console of <node> in lab <lab>. Escape: Ctrl+] or ~. at
  the start of a line.`
- on exit: `Connection to the console of <node> closed.`, after an empty line when an escape
  ended the session, as the cursor is then usually on the node's prompt. When the server ended
  it, its reason follows, also after an empty line: `Connection to the console of <node>
  closed: Taken over by <user> from <address> since <time> UTC.`, `... closed: The node's
  console closed: the node stopped.`, or `... closed: Your login session ended. Log in again
  with 'sherpa login'.`

`Ctrl+]` disconnects anywhere and is not sent to the node; `~.` disconnects at the start of a
line and `~~` sends one `~`. Either way, and when the server closes the session, the exit code
is 0. `sherpa ssh` takes only `~.`, so `Ctrl+]` still reaches a `telnet` run inside an SSH
session. A cursor position report (`ESC [ row ; column R`), the terminal's answer to busybox
`ash` asking for the cursor after each prompt, does not move the start of a line (Jato 0.5.1),
so `~.` after an Alpine prompt still disconnects.

## Errors

A refused upgrade leaves `tungstenite::Error::Http` in the error chain, with the status and the
body. `open_error` finds it under the connect context (an `OpenError::Refused`, with the status)
and `refusal_message` gives, with `<console>` being `console` for `sherpa console` and `VNC
console` for `sherpa vnc`:

| Answer | Message |
|---|---|
| 401 | `Not logged in to the Sherpa server, or the login has expired: run 'sherpa login'.` |
| any status with an API error body | the server's words, as they are: the body's `details` when it has them (a 400's `message` is only `Invalid request`), otherwise its `message`. For example `Access denied: only the owner of lab '<id>' can open its consoles`, `The console of <node> is in use by <user> from <address> since <time> UTC. Close that session, or take it over with 'sherpa console <node> --force'.`, `The VNC console of <node> is in use by ... 'sherpa vnc <node> --force'.`, `<node> is a container, and containers have no serial console. Connect with 'sherpa ssh <node>' instead.`, `<node> is a unikernel, and unikernels have no VNC console. Use 'sherpa console <node>' instead.`, `Node '<node>' is not running. Start it with 'sherpa resume'.`, `Node '<node>' was started before its VNC console moved behind Sherpa. ...` |
| 404 with no API error body | `The Sherpa server has no <console> route: it is older than this client. Upgrade the server.` |
| any other status | `The Sherpa server refused the <console> of <node> (HTTP <status>).` |

Any other failure to open, such as a timeout or a TLS error, gives `Failed to open the
<console> of <node> through the Sherpa server` with the cause kept (`OpenError::Failed`). A failure once open gives `The
console of <node> failed`, with Jato's `Error::Transport` as the cause.

## sherpa vnc

`cmd::vnc::vnc` relays a VM's VNC console to a VNC viewer on the user's machine. It replaces
the `ssh -L` tunnel to the server's VNC port that users ran before: the VNC console is now a
socket on the server that only sherpad reaches.

1. The lab's identity, the route (`console_url` with `Route::Vnc`) and the token are found as
   for `sherpa console`. `--force` is sent with every connection, so each takes the console over
   from a session of the user's own.
2. `check` asks the server first, at the route with `?check=true` (see
   [`SERVER.md`](SERVER.md#the-console-routes)), through `WebSocketClient::open` with the token
   and TLS settings every connection uses. The server answers a check with plain HTTP, not the
   upgrade, so the answer comes back as the handshake's `tungstenite::Error::Http`, read by
   `console::http_answer`, as a refusal is: the client has no HTTP client of its own, and this
   needs none. A `200` passes, with the `ConsoleCheck` body naming who holds the console, if
   anyone; a body that did not arrive with the headers only loses the holder. Any refusal ends the
   command before a port is opened, with the server's words (`refusal_message`) and exit code 1,
   as a refusal a viewer met ended it before: not logged in, not the owner (admins included), no
   such node, a container or unikernel, a node not running or still on its old VNC port. A server
   it cannot reach ends it too. A held console is not a refusal: after the address,
   `The VNC console of <node> is in use by <holder>: viewers are refused until that session ends,
   or take it over with 'sherpa vnc <node> --force'.` is printed on stderr (with `--force`: `...:
   a viewer connecting takes it over.`), and the command listens.
3. A `TcpListener` on `127.0.0.1`, port 0, so the system picks a free port. The address is
   printed on stdout: `VNC console of <node> in lab <lab>: connect a VNC viewer to
   127.0.0.1:<port>. Ctrl+C to stop.`
4. Each connection accepted is served in a task (`serve_viewer`): it opens the VNC route with
   `WebSocketClient::open`, as `sherpa console` does, then `relay` copies the viewer's reads (up
   to 16 KiB) to binary frames and binary frames to the viewer, until either side closes. The
   WebSocket is opened per connection, so a viewer that reconnects gets a new session, and the
   console is held only while a viewer is connected.
5. Ctrl+C (`tokio::signal::ctrl_c`, on Windows as well) stops listening and ends the command
   with code 0 and `Stopped relaying the VNC console of <node>.` on stderr; open connections end
   with it. The Ctrl+C future is made once and polled before the address is printed, and
   `serve_until` waits on that one future: a new one per loop turn would miss a Ctrl+C that
   came while another branch ran, such as a viewer's relay ending (it did, 7 times in 10).

Lines on stderr, per connection: `Viewer <address> connected to the VNC console of <node>.`,
then `Viewer <address> disconnected from the VNC console of <node>.` when the viewer closed, or
`The VNC console of <node> closed for viewer <address>: <reason>` when the server did.

A refusal after the check is told per connection, and the viewer's connection is closed with
nothing sent: the console taken by another session meanwhile, the node stopped, or the login
ended. `OpenError::lasting` decides what follows: a 409 (the console held by another session, a node
not running or still on its old VNC port) is printed and the command keeps listening, as the
next connection may succeed; any other refusal (401, 403, 404, 400) ends the command with the
message and a failing exit code, as every connection would meet it. A connection that fails to
open for another reason, such as a timeout, is printed and the command keeps listening.

The listener is on loopback, but other accounts on the user's machine can reach it while it
runs, as they could the `ssh -L` port it replaces. QEMU's VNC server asks for no password.

## Windows

Nothing in the console or in `sherpa vnc` is platform specific: the WebSocket is Tokio and rustls, and
`pass_through` runs the same Jato loop as `sherpa ssh`, with VT input and output modes on
Windows. `Ctrl+]` arrives as byte `0x1D` under VT input.

Test Windows changes in a desktop terminal (Windows Terminal and conhost), not only over SSH: a
desktop session differs from an `sshd` one. A Windows desktop session sets `USERNAME` but not
`USER` or `LOGNAME`, which broke the old SSH hop before Jato 0.5.1 while tests driven over SSH
passed.

## Tests

- Unit tests in `console.rs`: the route made from the server URL, for both consoles, a node name
  kept to one path segment, each refusal message, which refusals are lasting, a refused
  handshake found under the connect context, a connection failure keeping its cause, and the
  two lines. In `vnc.rs`: the listening, in-use and close lines, `check` against a local server
  answering as sherpad does (each refusal, 403, 400, 409 not running and still on a VNC port,
  401, told in the server's words with exit code 1, the request carrying `check=true` and the
  bearer token; a `200` with and without a holder; a server that cannot be reached), `serve_until` ending on an interrupt that
  comes as a viewer ends (50 times) and on a lasting refusal, and `relay` against a local WebSocket
  server: the RFB banner and the viewer's reply passed each way, the WebSocket closed when the
  viewer leaves, and the server's close reason ending the viewer's connection. `cli.rs` tests
  `--force` on both commands.
- The server's side is tested where it lives; see [`SERVER.md`](SERVER.md#node-consoles).
- The console lab test, `tests/labs/console/`, a pytest module, runs `sherpa console` in a
  pseudo-terminal against an Alpine VM with no `telnet` or `ssh` on PATH, as the owner and
  another user, and `sherpa vnc` with a scripted viewer that reads the RFB banner, as the owner,
  another user and an admin. An admin refused `sherpa console` is the owner login lab's. See its
  README.
