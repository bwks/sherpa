# Private bridge lab test

Tests `[[bridges]]` private bridges end to end through the `sherpa` CLI, with a VM and
containers in the same bridge.

| Node | Model | Connections |
|---|---|---|
| `dev01` | `alpine_linux` 3.23.0 VM, 4 data interfaces | eth1 in the bridge, eth2 link to fr03 |
| `fr01` | `frr_linux` 10.5.2 container | eth1 in the bridge, eth2 link to fr02 |
| `fr02` | `frr_linux` 10.5.2 container | eth1 in the bridge, eth2 link to fr01 |
| `fr03` | `frr_linux` 10.5.2 container | eth1 in the bridge, eth2 link to dev01 |
| `fr04` | `frr_linux` 10.5.2 container | eth1 in the bridge, no link |

Every link is a P2p (eBPF) link. A container on a link or a bridge takes all its data
interfaces as veths created in its namespace, its bridge member included, whose host end is a
port of the bridge. fr04 is on the bridge with no link, so it covers a container that has
veths only because of its bridge.

## Requirements

- A Sherpa server with the `alpine_linux` 3.23.0 and `frr_linux` 10.5.2 images. The run fails
  early if either is missing.
- The `sherpa` CLI logged in, with a device password set (`sherpa server user device-password`), plus `ssh`.

## Running

The runner is a pytest module; `uv run` installs the pytest version pinned in its script header
and runs pytest on it, passing on any other arguments:

```sh
uv run tests/labs/private-bridge/private_bridge_lab_test.py
uv run tests/labs/private-bridge/private_bridge_lab_test.py --keep-failed
uv run tests/labs/private-bridge/private_bridge_lab_test.py -k private_bridge_001
uv run tests/labs/private-bridge/private_bridge_lab_test.py --collect-only -q
uv run tests/labs/private-bridge/private_bridge_lab_test.py --self-test  # runner unit tests, seconds
```

pytest runs it directly too, for example `uvx --python 3.15 pytest==9.2.0
tests/labs/private-bridge/private_bridge_lab_test.py`. `--keep-failed` is defined in
`conftest.py` next to the runner. What the lab runners share, such as running and logging a
command, is in [`../labkit.py`](../labkit.py).

The module-scoped `prepared` fixture checks the requirements and writes the run's manifest under a
unique lab name; the `lab` fixture brings the lab up once and destroys it at the end, even when a
case fails, unless `--keep-failed` is given and something failed; the `addressed` fixture waits
for every node over SSH and addresses its bridge member and link ends, once for the module. The
cases are the functions named `test_<case_id>_*`, in the order below; every case runs even when
one fails. The last one destroys the lab itself; when it does not run, the `lab` fixture's
teardown destroys it and checks only that `sherpa inspect` no longer finds it. The runner's own
unit tests, under `--self-test`, are not collected by pytest.

Each run logs every command and its output under
`.tmp/lab-tests/private-bridge-<id>/lab/commands.log`. A run takes about 5–10 minutes.

## What it checks

It holds the private bridge cases of [`tests/specs/links-and-bridges.md`](../../specs/links-and-bridges.md).
Before each ping round every node flushes its neighbours, so a redeployed node's new MAC
addresses are learnt at once; every ping is sent from the source's own interface.

| Case | Check |
|---|---|
| `private_bridge_001` | After up, every bridge member (dev01, fr01, fr02, fr03 and fr04) pings every other member from its eth1 |
| `private_bridge_002` | fr01 and fr03 (on links), fr04 (on no link) and dev01 (VM) are redeployed in turn. After each, the redeployed node is addressed again, and every bridge ping of `private_bridge_001` and every link ping passes again |
| `private_bridge_003` | fr01, fr02, fr03 and dev01, each with a bridge member on eth1 and a link on eth2, ping every other bridge member from eth1 and their link peer from eth2 |
| `link_cleanup_002` | `sherpa destroy` succeeds, `sherpa inspect` no longer finds the lab, and no libvirt domain or network, Docker network or host interface whose name holds the lab's ID is left. When a listing cannot run here, such as `virsh` off the Sherpa server, the case is skipped after the others pass, naming what was not checked. It is skipped when `--keep-failed` is given and an earlier case failed |
