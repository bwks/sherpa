# NixOS lab test

Tests `nixos_linux` nodes end to end through the `sherpa` CLI. The image is built from
[`images/nixos`](../../../images/nixos).

| Node | Model | Connections |
|---|---|---|
| `nix01` | `nixos_linux` 26.05 VM, with a text file and an SSH provisioner | eth1 link to nix02 |
| `nix02` | `nixos_linux` 26.05 VM | eth1 link to nix01 |

`provision_current_user` is on, so the checks run as the lab owner.

## Requirements

- A Sherpa server with the `nixos_linux` 26.05 image, built from `images/nixos` and imported
  with `sherpa server image import --model nixos_linux --version 26.05`. The run fails early if
  it is missing.
- The `sherpa` CLI logged in, with an SSH key registered, plus `ssh`.

## Running

```sh
python3 tests/labs/nixos-linux/nixos_linux_lab_test.py
python3 tests/labs/nixos-linux/nixos_linux_lab_test.py --keep-failed
python3 tests/labs/nixos-linux/nixos_linux_lab_test.py --self-test  # runner unit tests, seconds
```

Each run logs every command and its output under
`.tmp/lab-tests/nixos-linux-<id>/lab/commands.log`. A run takes about 5 minutes.

## What it checks

On each node:

- `cloud-init status --wait` finishes cleanly (exit 0, no warnings);
- the SSH host key Sherpa pinned is the node's ed25519 key on disk, so it survives an sshd
  restart;
- the lab owner logs in with `/run/current-system/sw/bin/bash` (NixOS has no `/bin/bash`), has a
  group of its own, is in `wheel` and has passwordless sudo;
- the interfaces are named `eth0`, `eth1`, ..., password logins are allowed, and no systemd unit
  has failed.

Then:

- the text file is owned by `sherpa:sherpa` with mode 600, which needs the per-user groups the
  image turns on;
- `sherpa up` ran the SSH provisioner's script as the lab owner. The provisioner first waits on
  `cloud-init status --wait`, which needs the `cloud-init` command the image installs;
- both link ends ping each other;
- `sherpa destroy` cleans up and the lab is gone. Run on the Sherpa server, it also checks no
  libvirt domain or host interface of the lab is left.
