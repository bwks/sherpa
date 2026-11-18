# Console client

How `sherpa console` opens a lab node's serial console. The command is in
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
   `/api/v1/labs/<lab-id>/nodes/<node>/console`, each part one path segment, and `?force=true`
   with `--force`.
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
body. `open_error` finds it under the connect context and `refusal_message` gives:

| Answer | Message |
|---|---|
| 401 | `Not logged in to the Sherpa server, or the login has expired: run 'sherpa login'.` |
| any status with an API error body | the server's words, as they are: the body's `details` when it has them (a 400's `message` is only `Invalid request`), otherwise its `message`. For example `Access denied: only the owner of lab '<id>' can open its consoles`, `The console of <node> is in use by <user> from <address> since <time> UTC. Close that session, or take it over with 'sherpa console <node> --force'.`, `<node> is a container, and containers have no serial console. Connect with 'sherpa ssh <node>' instead.`, `Node '<node>' is not running. Start it with 'sherpa resume'.`, `Node '<node>' was started before consoles moved behind Sherpa. ...` |
| 404 with no API error body | `The Sherpa server has no console route: it is older than this client. Upgrade the server.` |
| any other status | `The Sherpa server refused the console of <node> (HTTP <status>).` |

Any other failure to open, such as a timeout or a TLS error, gives `Failed to open the console
of <node> through the Sherpa server` with the cause kept. A failure once open gives `The
console of <node> failed`, with Jato's `Error::Transport` as the cause.

## Windows

Nothing in the console is platform specific: the WebSocket is Tokio and rustls, and
`pass_through` runs the same Jato loop as `sherpa ssh`, with VT input and output modes on
Windows. `Ctrl+]` arrives as byte `0x1D` under VT input.

Test Windows changes in a desktop terminal (Windows Terminal and conhost), not only over SSH: a
desktop session differs from an `sshd` one. A Windows desktop session sets `USERNAME` but not
`USER` or `LOGNAME`, which broke the old SSH hop before Jato 0.5.1 while tests driven over SSH
passed.

## Tests

- Unit tests in `console.rs`: the route made from the server URL, a node name kept to one path
  segment, each refusal message, a refused handshake found under the connect context, a
  connection failure keeping its cause, and the two lines. `cli.rs` tests `--force`.
- The server's side is tested where it lives; see [`SERVER.md`](SERVER.md#node-consoles).
- The console lab test, `tests/labs/console/`, runs `sherpa console` in a pseudo-terminal
  against an Alpine VM with no `telnet` or `ssh` on PATH, as the owner, another user and an
  admin. See its README.
