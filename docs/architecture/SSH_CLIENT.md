# SSH client

How `sherpa ssh` opens an interactive shell on a lab node. The command is in
`crates/client/src/cmd/ssh.rs`, connecting and prompting in `crates/client/src/ssh_client/`,
and the shell in `crates/client/src/remote_shell/`. SSH itself
is the Jato library (`bwks/jato-lib`), so the client runs no `ssh` program.

## Connecting

`cmd::ssh::ssh` first runs the existing checks: the node is in the manifest, the lab files
exist, the alias is in `sherpa_ssh_config`, and the shared key has safe permissions. Then:

1. `jato::SshConfig::load_file("sherpa_ssh_config")` reads the lab's configuration, as
   `ssh -F` does, and `route(alias)` resolves the node and its `ProxyJump` to the Sherpa
   server.
2. `ssh_client::connect` connects along the route (keepalives every 30 s, 3 missed), with
   prompts on every hop (`PromptScope::AllHops`).
3. `remote_shell::run_shell` runs the shell until it ends, and `ssh` exits with its code.

The server hop has no `StrictHostKeyChecking` in the lab config, so Jato checks it strictly
against `~/.ssh/known_hosts`. Lab nodes have `StrictHostKeyChecking no`, so their keys are
not checked.

## The server host key prompt

`ssh_client::connect` asks as OpenSSH's `StrictHostKeyChecking ask` does, without
asking inside the SSH handshake:

```text
route.connect (strict)
  |
  +- ok -----------------------------------------------> session
  +- HostKeyUnknown { node, fingerprint, key, files }
  |    ask "continue connecting (yes/no/[fingerprint])?"
  |      no  --> error "Host key verification failed"
  |      yes --> KnownHosts::record(node, key) in the first file
  |              route.connect (strict) again ---------> session
  |              (a different key now is HostKeyChanged)
  +- any other error (changed, revoked, auth) ---------> error
```

Recording the exact key from the refusal means the second connection accepts only that key:
a server that swapped keys in between is refused.

## Passphrases and passwords

`ssh_client::connect` gives each hop it prompts on a `Prompter` from
`ssh_client::PromptCache::terminal`, so Jato asks on the terminal, without echo
(`rpassword`), as `ssh` does:

- `Enter passphrase for key '<path>': ` for an encrypted identity file, asked again after a
  wrong one, up to three times;
- `<user>@<host>'s password: ` when no key logged in and the server takes passwords, up to
  three times.

An empty answer declines. Prompts happen while connecting, before the terminal goes into raw
mode. The connecting and prompting code is `crate::ssh_client`, shared with the SSH
provisioners (see [`PROVISIONERS.md`](PROVISIONERS.md)), which ask only about the jump to the
server and keep answers for the run in a `PromptCache`. While a prompt may be shown, the
authentication limit is `konst::SSH_PROMPT_AUTH_TIMEOUT` (300 s), so typing is not cut off.

## The terminal

`remote_shell::session::run_shell` opens a Jato `Shell` with the local `TERM`
(`xterm-256color` if unset) and window size, then puts the terminal in raw mode
(`terminal::RawMode`, restored on drop, so every way out restores it) and loops:

| Source | Action |
| ------ | ------ |
| `Shell::next_event` | output to stdout; `Exit` kept for the exit code; `None` ends the loop |
| typed bytes | through `EscapeFilter`, then `Shell::send_bytes`; `~.` ends the loop |
| `ResizeWatcher::changed` | `Shell::resize` |

- Typed bytes are read on a thread of their own (`read_keys`), as reading the terminal blocks.
  A read still waiting when the shell ends is left behind and ends with the process.
- Raw mode passes every key as the terminal sends it, so control keys, arrows and Ctrl+C reach
  the node unchanged. On Windows, `windows_console` also turns on the console's
  virtual-terminal input and output, so keys arrive as the same VT bytes a Unix terminal
  sends and the node's escape sequences are drawn. There is one input path for every OS.
- Resizes come from `SIGWINCH` on Unix. Windows has no signal, so the size is polled every
  250 ms.
- `EscapeFilter` (`escape.rs`, pure) recognises `~.` and `~~` only at the start of a line, as
  OpenSSH does, including across reads.
- The exit code is the shell's if it fits in a byte, else 255, as for `~.` or a shell that
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
