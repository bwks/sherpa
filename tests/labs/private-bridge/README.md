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

Every link is a P2p (eBPF) link. A container with a link takes all its data interfaces as
veths created in its namespace, its bridge member included (fr01, fr02, fr03). fr04 has no
link, so its bridge member and disabled interfaces are Docker networks, attached in interface
order: a bridge member left out would rename every interface after it.

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
- the same after redeploying fr01 and fr03 (veths), fr04 (Docker networks) and dev01 (VM);
- `sherpa destroy` cleans up everything and the lab is gone. Run on the Sherpa server, it also
  checks no libvirt domain or network, Docker network or host interface of the lab is left.
