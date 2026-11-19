# Lab tests

Each directory here is a lab under test: one or more Sherpa manifests plus a runner that drives
the `sherpa` CLI against an installed server, the way a user does. This is different from
`tests/environments/`, which holds the disposable servers that the VM suites install Sherpa on.

| Lab | Covers | Time |
|---|---|---|
| [private-bridge](private-bridge/README.md) | `[[bridges]]` with a VM and containers, through up, redeploy and destroy | about 5–10 min |
| [p2p-containers](p2p-containers/README.md) | P2p links on network OS, application and OS containers, some without `ip` or a shell, through up, redeploy and destroy | about 2–5 min |
| [container-ztp](container-ztp/README.md) | The `ztp` setting on cEOS, SR Linux and FRR containers: config mounts and the lab owner's login with ZTP on, neither with it off, through up, redeploy and destroy | about 5–15 min |
| [cisco-iol](cisco-iol/README.md) | `cisco_iol` routers built from your own CML image: generated config and SSH, P2p links, TCP into IOL, `write memory` across a cold boot. Needs the IOL image | about 3–5 min |
| [p2p-links](p2p-links/README.md) | P2p (eBPF) link requirements on VM ⟷ VM, VM ⟷ container and container ⟷ container: L2 protocols, LLDP and LACP bundles, capture, per-direction impairment, jumbo frames, lifecycle | about 15–20 min |
| [nixos-linux](nixos-linux/README.md) | `nixos_linux` VMs built from [bwks/nixos-image](https://github.com/bwks/nixos-image): lab owner login, sudo, text files, cloud-init, a P2p link | about 5 min |
| [console](console/README.md) | `sherpa console` in a pseudo-terminal through sherpad, with no system `telnet` or `ssh`: login prompt, `Ctrl+]` and `~.`, a console in use naming its holder, `--force`, another user and an admin refused, a container refused | about 3–5 min |
| [ssh-access](ssh-access/README.md) | How `sherpa ssh` and SSH provisioners reach a node: directly, or through the Sherpa server with `--ssh-jump`, told apart by which process holds the connection the node sees. Needs a route to the lab's management network | about 2 min |
| [owner-login](owner-login/README.md) | The lab owner's login on a Linux VM and a network OS container, by key and by device password, with admin rights and no old `sherpa` login; what a second, admin account may not do: connect, download, add the owner a key. Needs an admin account | about 5–10 min |
| [talos-linux](talos-linux/README.md) | Sherpa-built Talos Kubernetes clusters | about 25–35 min |

## Rules

- A lab test uses only the `sherpa` CLI and tools inside the lab. It never calls server
  internals or reads the server's database. The exception is a lab whose nodes cannot be
  reached that way (no shell, no SSH server): its runner runs on the Sherpa server and uses
  Docker to reach them, and its README says so.
- Every run works in its own directory under `.tmp/lab-tests/` and gives each lab a unique name,
  so the checked-in manifests are never modified.
- A run always destroys its labs, including on failure, Ctrl-C and SIGTERM, unless
  `--keep-failed` is given. After destroying a lab it checks nothing was left behind.
- Pin every version: node images, and tool downloads with SHA-256 checksums.
- Files `sherpa up` and `sherpa download` write, all in the lab folder's `.sherpa/` (SSH key and
  config, `lab-info.toml`, cluster credentials), are git-ignored by the root `.gitignore`,
  including when a lab is run by hand inside its folder. Lab manifests are the exception and are tracked.
- Every runner's preflight stops unless the CLI is logged in and the user has a device password
  (`sherpa server user info` shows `Device password: set`): `sherpa up` is refused without one.
  Set it once with `sherpa server user device-password`.
  A lab with two accounts (owner-login) checks both have a device password, and takes the second
  account's login and passwords from options and environment variables, never printing them.
- Each runner has a `--self-test` mode for its pure logic, which runs in seconds.

Lab tests are started by hand. How and when end-to-end tests run is being planned in
[issue #330](https://github.com/bwks/sherpa/issues/330).
