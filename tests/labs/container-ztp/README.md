# Container ZTP lab test

Tests the ZTP setting on container nodes end to end through the `sherpa` CLI. Each model whose
config Sherpa mounts runs twice: with its image's setting (`volume`), and with
`ztp = "disabled"` in the manifest.

| Node | Model | ZTP |
|---|---|---|
| `ceos01`, `ceos02` | `arista_ceos` 4.35.1F | image's, `disabled` |
| `srl01`, `srl02` | `nokia_srlinux` 26.7.2 | image's, `disabled` |
| `ptx01`, `ptx02` | `juniper_cevolved` 26.2R1.7 | image's, `disabled` |
| `frr01`, `frr02` | `frr_linux` 10.5.2 | image's, `disabled` |
| `nats01` | `nats_bus` 2.11 | none to set: an application container |

The lab has no links.

## Requirements

- Run on the Sherpa server, as a user who can use Docker. A node with ZTP off has no `sherpa`
  login, and FRR with ZTP off has no SSH server, so the runner reads each node's container state
  and mounts with `docker inspect`.
- The node images above, pulled or imported with `sherpa server image pull` or
  `sherpa server image import`. The run fails early if one is missing. `arista_ceos` and
  `juniper_cevolved` are not in a public registry; on a server without them, run the other
  models with `--models`.
- The `sherpa` CLI logged in, `ssh` and `ping`.
- Internet access from the lab: the FRR node with ZTP on installs its SSH server at boot.

## Running

```sh
uv run tests/labs/container-ztp/container_ztp_lab_test.py
uv run tests/labs/container-ztp/container_ztp_lab_test.py --models frr_linux,nokia_srlinux,nats_bus
uv run tests/labs/container-ztp/container_ztp_lab_test.py --keep-failed
uv run tests/labs/container-ztp/container_ztp_lab_test.py --self-test  # runner unit tests, seconds
```

Each run logs every command and its output under
`.tmp/lab-tests/container-ztp-<id>/lab/commands.log`.

## What it checks

- `sherpa validate` refuses `ztp = "cdrom"` on a container, naming the node and the settings its
  model takes, and accepts the lab's manifest;
- a node with ZTP on has every config mount of its model and takes an SSH login as `sherpa`
  through the lab's SSH config;
- a node with ZTP off is running, has none of those mounts, and does not take an SSH login as
  `sherpa`, for the reason its model has. `srl02` has no management address once it has booted:
  SR Linux as shipped never configures the address its container was given, so the SSH
  connection finds no route and the address does not answer a ping. While it boots, for about a
  minute, the image's SSH server answers there and refuses the login; the run waits for the
  booted state and fails if a login ever succeeds. `frr02` answers a ping on its address and refuses the
  SSH connection, because nothing installed an SSH server. The cEOS and cJunosEvolved nodes
  must refuse the login too; the run prints the reason it saw;
- every node, the application container included, is still running, never restarted, after the
  others have booted;
- the same after redeploying one node with ZTP on and one with it off;
- `sherpa destroy` cleans up everything: no container, Docker network or host interface of the
  lab is left.
