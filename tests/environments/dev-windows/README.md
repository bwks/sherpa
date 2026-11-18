# Sherpa Windows dev box

A Windows 11 node for Windows checks of the Sherpa client, such as
`crates/client/src/windows_acl.rs`. It runs on the Sherpa host itself, as lab `win-test`, node
`win`, not inside a [dev server](../dev/README.md): a VM in a dev server is nested, and Windows
is too slow there. It stays up between tasks and is reused: do not destroy it.

## Who runs Windows checks

The host runs them, on this box. Agents don't use a Windows VM.

- When a change needs a Windows check, the agent cross-compiles the client on its dev server
  (see [Build the client for Windows](#build-the-client-for-windows)), leaves `sherpa.exe` in
  `/var/tmp`, and says so in its report. The host copies it to the box and checks it there.
- Only some changes need one: changes to the interactive terminal (Jato's console code,
  `sherpa console`, `sherpa ssh`) and the installer. CI covers the rest.
- Never install Rust on the box or build there.

This folder's `manifest.toml` is what the host's box was made from; it is kept as its record.
`provision/setup.ps1`, which installed Rust on the box, is no longer used.

## Node

| Setting | Value |
|---|---|
| Lab, node | `win-test`, `win`, on the Sherpa host |
| Image | `devbox_windows` `windows11-20261030`, built with the [packer template](https://github.com/bwks/packer-devbox-windows) |
| vCPUs | 8 |
| Memory | 16 GiB |

The node's login is the lab owner's account, with their SSH keys and device password. It is
reached through the lab's SSH config on the host, `ssh -F .sherpa/ssh/config win.<lab-id>`
from the lab's folder, where `<lab-id>` is the `id` in `.sherpa/lab-info.toml`.

## Build the client for Windows

On the dev server, which has the `x86_64-pc-windows-gnu` Rust target and MinGW:

```bash
cargo build -p sherpa --target x86_64-pc-windows-gnu
cargo clippy -p sherpa --target x86_64-pc-windows-gnu --all-targets -- -D warnings
cargo test -p sherpa --target x86_64-pc-windows-gnu --no-run
```

The client is `target/x86_64-pc-windows-gnu/debug/sherpa.exe`; an agent copies it to
`/var/tmp/sherpa.exe` for the host. `cargo test --no-run` prints the path of each test
executable it builds. Only the client builds for Windows: the server,
`libvirt` and `network` crates are Linux only.

## Test the client on the box

The host does this. It copies `sherpa.exe`, and any test executable to run, to the box over
SSH, then runs them there:

```bash
scp -F .sherpa/ssh/config sherpa.exe win.<lab-id>:sherpa.exe
ssh -F .sherpa/ssh/config win.<lab-id> '.\sherpa.exe --version'
```

## Test in a desktop terminal too

A session over SSH is not what a Windows user has: Win32-OpenSSH's `sshd` sets `USER` and
`LOGNAME`, which Windows Terminal and conhost do not, and its console is a pseudo console. Check
interactive commands such as `sherpa ssh` and `sherpa console` in a desktop terminal as well,
for example by signing in on the node's display (`virsh send-key` and `virsh screenshot` on
the server can drive it).

## Destroy it

Only when the user asks, for example to rebuild it from a new image. On the host, from the
lab's folder:

```bash
sherpa destroy
```
