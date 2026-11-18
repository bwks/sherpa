# Sherpa Windows dev box

A Windows 11 node for building and testing the Sherpa client's Windows code, such as
`crates/client/src/windows_acl.rs`, which the Linux [dev server](../dev/README.md) cannot
compile. It stays up between tasks and is reused: do not destroy it after use. Bring it up only
if it is down.

## Node

| Setting | Value |
|---|---|
| Image | `devbox_windows` `windows11-20261030`, built with the [packer template](https://github.com/bwks/packer-devbox-windows) |
| vCPUs | 8 |
| Memory | 16 GiB |

The node's login is your own account, as the lab owner, with your SSH keys and your device
password.

## Bring it up

Only if it is down: `sherpa inspect` in this folder shows whether the lab is running. From this
folder, on a Sherpa server that has the image. Set your device password first, once:
`sherpa up` is refused without one. It asks for your Sherpa login password, then the device
password twice.

```bash
sherpa server user device-password
sherpa up
```

SSH provisioners do not run on Windows nodes, so the toolchain is installed over SSH once the
node is up. `provision/setup.ps1` installs Rust from the devbox's mise config:

```bash
ssh -F .sherpa/ssh/config win.<lab-id> 'powershell -NoProfile -Command -' < provision/setup.ps1
```

`<lab-id>` is the `id` in `.sherpa/lab-info.toml`. Expect about 5 minutes for Windows to be ready and a
few minutes for Rust.

## Build and test the client

Send the working tree, including uncommitted changes, from a checkout:

```bash
git ls-files -co --exclude-standard -z | tar --null -T - -czf /tmp/sherpa-src.tgz
scp -F .sherpa/ssh/config /tmp/sherpa-src.tgz win.<lab-id>:sherpa-src.tgz
ssh -F .sherpa/ssh/config win.<lab-id> 'powershell -NoProfile -Command "Remove-Item -Recurse -Force sherpa -ErrorAction SilentlyContinue; New-Item -ItemType Directory sherpa | Out-Null; tar -xzf sherpa-src.tgz -C sherpa"'
```

Then build, lint and test the client:

```bash
ssh -F .sherpa/ssh/config win.<lab-id> 'powershell -NoProfile -Command "cd sherpa; mise exec rust -- cargo clippy -p sherpa --all-targets -- -D warnings; mise exec rust -- cargo test -p sherpa"'
```

Only the client is built here: the server, `libvirt` and `network` crates are Linux only.

## Test in a desktop terminal too

A session over SSH is not what a Windows user has: Win32-OpenSSH's `sshd` sets `USER` and
`LOGNAME`, which Windows Terminal and conhost do not, and its console is a pseudo console. Check
interactive commands such as `sherpa ssh` and `sherpa console` in a desktop terminal as well,
for example by signing in on the node's display (`virsh send-key` and `virsh screenshot` on
the server can drive it).

## Destroy it

Only when the user asks, for example to rebuild it from a new image:

```bash
sherpa destroy
```
