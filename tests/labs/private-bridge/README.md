# Private bridge lab test

Tests `[[bridges]]` private bridges end to end through the `sherpa` CLI, with a VM and
containers in the same bridge.

| Node | Model | Connections |
|---|---|---|
| `dev01` | `alpine_linux` 3.23.0 VM, 4 data interfaces | eth1 in the bridge, eth2 P2p link to fr03 |
| `fr01` | `frr_linux` 10.5.2 container | eth1 in the bridge, eth2 link to fr02 |
| `fr02` | `frr_linux` 10.5.2 container | eth1 in the bridge, eth2 link to fr01 |
| `fr03` | `frr_linux` 10.5.2 container, P2p | eth1 in the bridge, eth2 P2p link to dev01 |

fr01's link comes after its bridge member, so a bridge member left out would rename the link
interface. fr03 has a P2p link, so its data interfaces are veths moved into the container
instead of Docker networks.

## Requirements

- A Sherpa server with the `alpine_linux` 3.23.0 and `frr_linux` 10.5.2 images. The run fails
  early if either is missing.
- The `sherpa` CLI logged in, plus `ssh`.

## Running

```sh
python3 tests/labs/private-bridge/private_bridge_lab_test.py
python3 tests/labs/private-bridge/private_bridge_lab_test.py --keep-failed
python3 tests/labs/private-bridge/private_bridge_lab_test.py --self-test  # runner unit tests, seconds
```

Each run logs every command and its output under
`.tmp/lab-tests/private-bridge-<id>/lab/commands.log`. A run takes about 5–10 minutes.

## What it checks

- every bridge member and link end gets an address, and every bridge member pings every other
  member, and each link end its peer, from that interface;
- the same after redeploying fr01 (Docker networks), fr03 (P2p) and dev01 (VM);
- `sherpa destroy` cleans up everything and the lab is gone. Run on the Sherpa server, it also
  checks no libvirt domain or network, Docker network or host interface of the lab is left.
