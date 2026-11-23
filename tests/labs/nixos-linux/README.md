# NixOS lab test

Tests `nixos_linux` nodes end to end through the `sherpa` CLI. The image is built from
[bwks/nixos-image](https://github.com/bwks/nixos-image).

| Node | Model | Connections |
|---|---|---|
| `nix01` | `nixos_linux` 26.05 VM, with a text file and an SSH provisioner | eth1 link to nix02 |
| `nix02` | `nixos_linux` 26.05 VM | eth1 link to nix01 |

The nodes' only login is the lab owner's own account, so the checks run as the lab owner.

## Requirements

- A Sherpa server with the `nixos_linux` 26.05 image, built from [bwks/nixos-image](https://github.com/bwks/nixos-image) and imported
  with `sherpa server image import --model nixos_linux --version 26.05`. The run fails early if
  it is missing.
- The `sherpa` CLI logged in, with an SSH key registered and a device password set
  (`sherpa server user device-password`), plus `ssh`. The run stops early when the password is not set.

## Running

The runner is a pytest module; `uv run` installs the pytest version pinned in its script header
and runs pytest on it, passing on any other arguments:

```sh
uv run tests/labs/nixos-linux/nixos_linux_lab_test.py
uv run tests/labs/nixos-linux/nixos_linux_lab_test.py --keep-failed
uv run tests/labs/nixos-linux/nixos_linux_lab_test.py -k owner_login_027
uv run tests/labs/nixos-linux/nixos_linux_lab_test.py --self-test  # runner unit tests, seconds
```

pytest runs it directly too, for example `uvx --python 3.15 pytest==9.2.0
tests/labs/nixos-linux/nixos_linux_lab_test.py`. `--keep-failed` is defined in `conftest.py` next
to the runner. What the lab runners share, such as running and logging a command, is in
[`../labkit.py`](../labkit.py).

The module-scoped `prepared` fixture checks the requirements and writes the run's manifest under a
unique lab name, with the text file and the provisioner's script beside it; the `lab` fixture
brings the lab up once and destroys it at the end, even when a case fails, unless `--keep-failed`
is given and something failed. Its teardown also checks that `sherpa inspect` no longer finds the
lab and, run on the Sherpa server, that no libvirt domain or host interface of the lab is left.
The `ready` fixture waits until every node takes SSH and cloud-init has finished, once for the
module. The cases are the functions named `test_<case_id>_*`, in the order below; none changes
the lab, and every case runs even when one fails. The runner's own unit tests, under
`--self-test`, are not collected by pytest.

Each run logs every command and its output under
`.tmp/lab-tests/nixos-linux-<id>/lab/commands.log`. A run takes about 5 minutes.

## What it checks

On each node, as the lab owner:

| Case | Check |
|---|---|
| [`config_delivery_003`](../../specs/ztp.md) | `cloud-init status --wait` finishes cleanly: exit 0, no warnings |
| [`node_host_keys_001`](../../specs/client-lab-files.md) | The SSH host key Sherpa pinned in `.sherpa/ssh/known_hosts` is the node's ed25519 key on disk, so it survives an sshd restart |
| [`owner_login_027`](../../specs/accounts-and-auth.md) | The lab owner's shell is `/run/current-system/sw/bin/bash` (NixOS has no `/bin/bash`), they have a group of their own, are in `wheel` and have passwordless sudo |
| [`node_interfaces_009`](../../specs/links-and-bridges.md) | The interfaces are named `eth0` and `eth1`, and both are up as the node booted, before anything configures them |
| [`boot_files_001`](../../specs/provisioning.md) | nix01's text file is owned by the lab owner and the owner's own group (`<owner>:<owner>`) with mode 600. The runner fills the manifest's `{owner}` with the owner's name, which needs the per-user groups the image turns on |
| [`provisioner_scripts_001`](../../specs/provisioning.md) | `sherpa up` ran nix01's SSH provisioner script as the lab owner, in their home. The provisioner first waits on `cloud-init status --wait`, which needs the `cloud-init` command the image installs |
