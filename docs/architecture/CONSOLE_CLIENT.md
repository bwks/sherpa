# Console client

How `sherpa console` opens a lab node's serial console. The command is in
`crates/client/src/cmd/console.rs`; the SSH hop to the server is `ssh_client::connect_server`
in `crates/client/src/ssh_client/connect.rs`. Telnet and the terminal pass-through are the
Jato library (`bwks/jato-lib`) with its `telnet` and `terminal` features, so the client runs no
`telnet` program.

## Where the console is

Each VM and unikernel has a QEMU chardev, `socket,host=<node loopback>,port=2323,telnet=on,
server=on,wait=off`, from the `<serial type='tcp'>` element with `<protocol type='telnet'/>` in
`libvirt_domain.jinja` and `unikernel_domain.jinja`. QEMU is the telnet server. It listens on
the node's address in the lab's loopback network (`127.127.x.N`) on the Sherpa server host,
port `konst::TELNET_PORT`. That address is reachable only from the server, so the client
always reaches it through an SSH session to the server, even when the client runs on the
server host. Containers have no such chardev.

## Connecting

`cmd::console::console`:

1. `console_node` finds the node in the manifest. A container (`NodeModel::kind`) is refused
   here, before anything connects. The console address is the node's place in the manifest
   in the lab's `loopback_network` from `lab-info.toml` (`boot01` is address 255), on
   `TELNET_PORT`.
2. It reads `.sherpa/ssh/config` and takes the first `Host` block's first alias. Every block
   has the same `ProxyJump`, the server, so `SshConfig::route(alias)` gives the server hop as
   the route's last jump host.
3. `ssh_client::connect_server` connects to that jump host through any jump hosts before it.
   It shares `attempt` with `ssh_client::connect`, which `sherpa ssh` uses, so the host key
   policy (`StrictHostKeyChecking` `ask` against `~/.ssh/known_hosts`, as in
   [`SSH_CLIENT.md`](SSH_CLIENT.md)), the `TerminalPrompter` in a `PromptCache` for key
   passphrases and passwords, `konst::SSH_PROMPT_AUTH_TIMEOUT`, and keepalives are the same.
   The node's own host key is not involved, so there is no `HostKeyRefresh`.
4. `Telnet::connect_via` opens a `direct-tcpip` channel from the server to the console, with
   `konst::CONSOLE_CONNECT_TIMEOUT` (10 s) to open it and `konst::CONSOLE_NEGOTIATION_TIMEOUT`
   (3 s) for QEMU's first negotiation.
5. `Telnet::interactive` passes the terminal through until the console closes or an escape is
   typed, then the telnet channel and the SSH session are closed.

There is no direct connection first. A client that is itself a Sherpa server could otherwise
reach a different lab's console at the same loopback address.

## Lines and escapes

Jato's banner is off (`InteractiveOptions::banner = false`). Sherpa prints its own lines on
stderr, with the node's name:

- after connecting: `Connected to the console of <node> in lab <lab>. Escape: Ctrl+] or ~. at
  the start of a line.`
- on exit: `Connection to the console of <node> closed.`, after an empty line when an escape
  ended the session, as the cursor is then usually on the node's prompt.

`Ctrl+]` disconnects anywhere and is not sent to the node; `~.` disconnects at the start of a
line and `~~` sends one `~`. Either way, and when the console closes, the exit code is 0.
`sherpa ssh` takes only `~.`, so `Ctrl+]` still reaches a `telnet` run inside an SSH session.

## Errors

`console_error` turns Jato's telnet errors into one message each, with Jato's error kept as
the cause:

| Jato error | Message |
|---|---|
| `TelnetRefused` (the server refused the channel or could not reach the console) | `The console of <node> refused the connection at <addr> through the Sherpa server. Is the node running?` |
| `TelnetTimeout` | `The console of <node> did not answer at <addr> within 10s.` |
| `TelnetInUse` (connected, then silence for 3 s: QEMU serves one client and says nothing to a second) | `The console of <node> is in use: another client holds it, and it said nothing within 3s. A console serves one client at a time; close the other session and try again.` |
| `TelnetClosed` | `The console of <node> closed the connection at <addr>. Is the node running?` |
| anything else, such as `Tunnel` | `Failed to open the console of <node> through the Sherpa server` |

A container gives `<node> is a container, and containers have no serial console. Connect with
'sherpa ssh <node>' instead.` A node not in the manifest gives `Device not found: <node>`.

## Windows

Nothing in the console is platform specific: the telnet client is Tokio TCP and SSH channels,
and `Telnet::interactive` runs the same Jato pass-through loop as `sherpa ssh`, with VT input
and output modes on Windows. `Ctrl+]` arrives as byte `0x1D` under VT input.

## Tests

- Unit tests in `console.rs`: the console address, the container refusal, the server alias,
  each error message, and the two lines. `ssh_client::connect` has a test for a route with no
  server hop.
- The console lab test, `tests/labs/console/`, runs `sherpa console` in a pseudo-terminal
  against an Alpine VM with no `telnet` on PATH. See its README.
