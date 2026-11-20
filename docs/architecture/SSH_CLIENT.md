# SSH client

How `sherpa ssh` opens an interactive shell on a lab node. The command is in
`crates/client/src/cmd/ssh.rs` and Sherpa's connection rules are in
`crates/client/src/ssh_client/`. SSH itself, the terminal prompts and the interactive shell
are the Jato library (`bwks/jato-lib`) with its `terminal` feature, so the client runs no
`ssh` program.

## Connecting

`cmd::ssh::ssh` takes the lab's ID from the folder (see
[`LAB_FOLDER.md`](LAB_FOLDER.md#lab-identity)), so it works in a folder made by `sherpa
download`, with no manifest, and for an OS user whose name is not the lab owner's. It first runs
the existing checks: the node is in the manifest, when the folder has one, the lab files exist,
the alias is in `.sherpa/ssh/config`, and the saved lab info is the lab's. No key file is checked:
nodes take the user's own keys. Then:

1. `jato::SshConfig::load_file(".sherpa/ssh/config")` reads the lab's configuration, as
   `ssh -F` does, and `ssh_client::node_route` resolves the node. The config has no
   `ProxyJump`: the route goes straight to the node. With `--ssh-jump`, `SshJump::apply` puts
   the Sherpa server in front of it, as `ssh -J` does (see [The jump](#the-jump)).
2. `ssh_client::connect` connects along the route (keepalives every 30 s, 3 missed), with a
   `jato::TerminalPrompter` wrapped in a `jato::PromptCache` on every hop
   (`PromptScope::AllHops`). A route with no jump is checked for a route to the node first
   (see [Unreachable nodes](#unreachable-nodes)).
3. `Session::interactive_shell` runs the shell until it ends, and `ssh` exits with
   `ShellEnd::exit_code`.

`ssh_client` holds only what is Sherpa's to decide: whether to add the jump (`SshAccess`,
`SshJump`), which hops may ask (`PromptScope`), whether a node reached directly has a route,
and, for provisioners, which failures are worth retrying (`retryable`).

## The jump

The lab's SSH config, built by sherpad (`services/ssh_config.rs`), reaches every node
directly. A client on the Sherpa server, or on the lab's tailnet with its routes accepted, needs
nothing else. A client with no route to the management network adds the Sherpa server as a jump
host for one command with `--ssh-jump`, on `sherpa ssh` and on the commands that run SSH
provisioners: `sherpa up`, `sherpa provision` and `sherpa redeploy`. Nothing is saved, in the
manifest or the lab folder.

`SshAccess::new` builds the jump when the flag is set: `SshJump::logged_in` takes the user from
the login token and the host from the server URL the command uses (an IPv6 address without its
brackets). `SshJump::apply` resolves that host in the lab config, which has no block for it, so
it gets OpenSSH's defaults (port 22, `StrictHostKeyChecking ask` against `~/.ssh/known_hosts`),
sets its user to the Sherpa user, and makes it the route's only jump, replacing any other, as
`ssh -J` does. The user needs an account of the same name on the server's host that takes their
key. Plain OpenSSH does the same with `ssh -F .sherpa/ssh/config -J <user>@<server>
<node>.<lab-id>`.

## Unreachable nodes

A node reached directly from a machine with no route to it would otherwise fail only after the
connect time limit, and a provisioner would retry it until the lab's ready timeout. Before a
route with no jump connects, `ssh_client::connect` opens a TCP connection to the node's SSH port,
for at most `konst::SSH_DIRECT_ROUTE_TIMEOUT` (10 s). An answer, or a refusal (the node itself
replied, so it is reachable and its SSH server is not up yet), lets the connection go ahead.
Anything else, such as no route, an unreachable host or no answer, fails at once with
`<node> at <address>:<port> is not reachable from this machine (<cause>). Use --ssh-jump to go
through the Sherpa server, or join the lab's tailnet and accept its routes.` That error is not a
Jato error, so `retryable` says no and provisioners stop at once. There is no fallback to the
jump. `sherpa console` makes no SSH
connection; it goes through sherpad's API (see [`CONSOLE_CLIENT.md`](CONSOLE_CLIENT.md)).

## Lines

Jato's banner is off (`InteractiveOptions::banner = false`), and Sherpa prints its own lines on
stderr, with the node's alias, so exactly one of each appears:

- once the shell is open: `Connected to <node>.<lab-id>. Escape: ~. at the start of a line.`
- when it ends: `Connection to <node>.<lab-id> closed.` After `~.` (`ShellEnd::escaped`) the
  line starts on a new line, as the cursor is usually on the node's prompt.

The console prints lines of the same form.

## The server host key question

With `--ssh-jump`, the server hop has no `StrictHostKeyChecking` in the lab config, so Jato uses OpenSSH's
default, `ask` (`HostKeyPolicy::Ask`), against `~/.ssh/known_hosts`. A lab node whose key the
server recorded has `StrictHostKeyChecking yes` and the lab's `.sherpa/ssh/known_hosts` file
(`HostKeyPolicy::Strict`), so it is never asked about. Other nodes have
`StrictHostKeyChecking no`, so their keys are not checked. See "Node readiness" in
[`SERVER.md`](SERVER.md) for how keys are recorded.

When the node's key is unknown or does not match (`HostKeyUnknown` or `HostKeyChanged` for
the target, not a jump host), `ssh_client::connect` asks the server for the node's current
key with `HostKeyRefresh` (the `node.host_keys` RPC), which saves the lab's `known_hosts`
file it returns, and connects once more. The server builds that file from the host keys on the
lab's node records and keeps no copy of it. A changed key prints a warning; an unknown one a
note. If the key still does not match, the connection fails. Answers to prompts are kept in
the `PromptCache`, so the second attempt does not ask again. `sherpa ssh` and the
provisioners both connect this way.

For an unknown server key, Jato refuses the handshake, asks the prompter
`continue connecting (yes/no/[fingerprint])?`, records the exact key from the refusal on
`yes` (or the fingerprint), prints OpenSSH's `Permanently added` warning, and runs the
handshake again, which accepts only that key. A changed or revoked key is refused without
asking. Asking happens between handshakes, so the connect time limit does not cut the person
off. Jato's `docs/architecture/connecting.md` has the details.

## Passphrases and passwords

`TerminalPrompter` asks on the terminal, without echo, as `ssh` does:

- `Enter passphrase for key '<path>': ` for an encrypted identity file, asked again after a
  wrong one, up to three times;
- `<user>@<host>'s password: ` when no key logged in and the server takes passwords, up to
  three times.

An empty answer declines. Prompts happen while connecting, before the terminal goes into raw
mode. The SSH provisioners (see [`PROVISIONERS.md`](PROVISIONERS.md)) use the same
`ssh_client::connect` with `PromptScope::NoNodePassword`: the node, which is the first hop
without a jump, may ask only for the passphrase of the user's key, never for a password, since a
node whose login is not ready yet must fail and be retried; the jump to the server may ask
anything. Answers are kept for the run in one `PromptCache`, so a passphrase is asked once. While a
prompt may be shown, the authentication limit is `konst::SSH_PROMPT_AUTH_TIMEOUT` (300 s), so
typing is not cut off.

## The terminal

`Session::interactive_shell` (Jato's `docs/architecture/terminal.md`) passes the local
terminal through, as `ssh` does:

- the local `TERM` (`xterm-256color` if unset) and window size, and resizes (`SIGWINCH` on
  Unix, a 250 ms poll on Windows);
- raw mode, restored however the shell ends, so control keys, arrows and Ctrl+C reach the
  node unchanged; on Windows the console is also switched to VT input and output;
- `~.` at the start of a line disconnects, `~~` sends one `~`.

The exit code is the shell's if it fits in a byte, else 255, as for `~.` or a shell that
reported none. Windows' OpenSSH server does not report PowerShell's exit code for an
interactive shell, so it is 0 there, as with the system `ssh`.

## Limits

- No agent, X11 or port forwarding, and no escapes other than `~.` and `~~`. `Ctrl+]` is the
  console's alone, so it still reaches a `telnet` run inside an SSH session.
- Tested on Linux with Ubuntu, Cisco IOSv and Windows Server nodes, and from a Windows Server
  2022 client in a ConPTY console (Windows Terminal and conhost both use it).

## Thread stack

The client runs on `#[tokio::main]`, so commands are polled on the main thread: 1 MiB on
Windows, and usually 8 MiB on Linux (`ulimit -s`). An SSH connection fits in that on
Windows, debug builds included, because Jato boxes its connection futures; before Jato
v0.4.2 a debug build overflowed it through a jump host, and the client ran its commands on
an 8 MiB thread instead. Jato's connecting architecture doc describes the stack use.
