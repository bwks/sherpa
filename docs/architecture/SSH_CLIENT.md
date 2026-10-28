# SSH client

How `sherpa ssh` opens an interactive shell on a lab node. The command is in
`crates/client/src/cmd/ssh.rs` and Sherpa's connection rules are in
`crates/client/src/ssh_client/`. SSH itself, the terminal prompts and the interactive shell
are the Jato library (`bwks/jato-lib`) with its `terminal` feature, so the client runs no
`ssh` program.

## Connecting

`cmd::ssh::ssh` first runs the existing checks: the node is in the manifest, the lab files
exist, the alias is in `sherpa_ssh_config`, and the shared key has safe permissions. Then:

1. `jato::SshConfig::load_file("sherpa_ssh_config")` reads the lab's configuration, as
   `ssh -F` does, and `route(alias)` resolves the node and its `ProxyJump` to the Sherpa
   server.
2. `ssh_client::connect` connects along the route (keepalives every 30 s, 3 missed), with a
   `jato::TerminalPrompter` wrapped in a `jato::PromptCache` on every hop
   (`PromptScope::AllHops`).
3. `Session::interactive_shell` runs the shell until it ends, and `ssh` exits with
   `ShellEnd::exit_code`.

`ssh_client` holds only what is Sherpa's to decide: which hops may ask (`PromptScope`) and,
for provisioners, which failures are worth retrying (`retryable`).

## The server host key question

The server hop has no `StrictHostKeyChecking` in the lab config, so Jato uses OpenSSH's
default, `ask` (`HostKeyPolicy::Ask`), against `~/.ssh/known_hosts`. A lab node whose key the
server recorded has `StrictHostKeyChecking yes` and the lab's `sherpa_known_hosts` file
(`HostKeyPolicy::Strict`), so it is never asked about. Other nodes have
`StrictHostKeyChecking no`, so their keys are not checked. See "Node readiness" in
[`SERVER.md`](SERVER.md) for how keys are recorded.

When the node's key is unknown or does not match (`HostKeyUnknown` or `HostKeyChanged` for
the target, not a jump host), `ssh_client::connect` asks the server for the node's current
key with `HostKeyRefresh` (the `node.host_keys` RPC), which saves the lab's `known_hosts`
file it returns, and connects once more. A changed key prints a warning; an unknown one a
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
`ssh_client::connect` but ask only about the jump to the server
(`PromptScope::JumpHostsOnly`), and keep answers for the run in one `PromptCache`. While a
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

- No agent, X11 or port forwarding, and no escapes other than `~.` and `~~`.
- Tested on Linux with Ubuntu, Cisco IOSv and Windows Server nodes, and from a Windows Server
  2022 client in a ConPTY console (Windows Terminal and conhost both use it).

## Thread stack

`main` runs the command line on a thread of its own with an 8 MiB stack, and the Tokio
runtime's threads get the same (`THREAD_STACK_SIZE` in `crates/client/src/main.rs`). Windows
gives a main thread 1 MiB, and an SSH connection's nested polls (Jato, `russh`, the jump hosts)
overflowed it in a debug build; Linux's main thread already has 8 MiB.
