# Container ZTP lab test

Tests the ZTP setting on container nodes end to end through the `sherpa` CLI. Each model whose
config Sherpa mounts runs twice: with its image's setting (`volume`), and with
`ztp.method = "disabled"` in the manifest.

| Node | Model | ZTP |
|---|---|---|
| `ceos01`, `ceos02` | `arista_eos` container 4.35.1F | image's, `disabled` |
| `srl01`, `srl02` | `nokia_srlinux` 26.7.2 | image's, `disabled` |
| `frr01`, `frr02` | `frr_linux` 10.5.2 | image's, `disabled` |
| `nats01` | `nats_bus` 2.11 | none to set: an application container |

The lab has no links.

cJunosEvolved (`juniper_evolved`, kind `container`) is not in the lab, though Sherpa mounts its
config too: its container boots a VM, which is nested in the dev servers, where these tests run,
and takes many minutes there. Its Junos config is covered by the template unit tests
(`crates/template/tests/juniper_junos`); its config mount and ZTP-off behaviour are not checked
live.

## Requirements

- Run on the Sherpa server, as a user who can use Docker. A node with ZTP off has no login
  for the lab owner, and FRR with ZTP off has no SSH server, so the runner reads each node's container state
  and mounts with `docker inspect`.
- The node images above, pulled or imported with `sherpa server image pull` or
  `sherpa server image import`. The run fails early if one is missing. `arista_eos`
  (`--kind container`) is not in a public registry; on a server without it, run the other
  models with `--models`.
- The `sherpa` CLI logged in, with a device password set (`sherpa server user device-password`)
  and an SSH key registered, `ssh` and `ping`. The run stops early when the password is not set.
- Internet access from the lab: the FRR node with ZTP on installs its SSH server at boot.

## Running

The runner is a pytest module; `uv run` installs the pytest version pinned in its script header
and runs pytest on it, passing on any other arguments:

```sh
uv run tests/labs/container-ztp/container_ztp_lab_test.py
uv run tests/labs/container-ztp/container_ztp_lab_test.py --models frr_linux,nokia_srlinux,nats_bus
uv run tests/labs/container-ztp/container_ztp_lab_test.py --keep-failed
uv run tests/labs/container-ztp/container_ztp_lab_test.py -k ztp_disabled_002
uv run tests/labs/container-ztp/container_ztp_lab_test.py --self-test  # runner unit tests, seconds
```

pytest runs it directly too, for example `uvx --python 3.15 pytest==9.2.0
tests/labs/container-ztp/container_ztp_lab_test.py`. `--keep-failed` and `--models` are defined in
`conftest.py` next to the runner.

The module-scoped `prepared` fixture checks the requirements and writes the run's manifest under a
unique lab name; the `lab` fixture brings the lab up once and destroys it at the end, even when a
case fails, unless `--keep-failed` is given and something failed; the `redeployed` fixture
redeploys one node with ZTP on and one with it off, of the first model that has both, once for the
module. The cases are the functions named `test_<case_id>_*`, in the order below; every case runs
even when one fails. The runner's own unit tests, under `--self-test`, are not collected by pytest.

Each run logs every command and its output under
`.tmp/lab-tests/container-ztp-<id>/lab/commands.log`.

## What it checks

It holds the container cases of [`tests/specs/ztp.md`](../../specs/ztp.md):

| Case | Check |
|---|---|
| `ztp_setting_001` | `sherpa validate` refuses `ztp.method = "cdrom"` on a container, naming the node and the settings its model takes, and accepts the lab's manifest. It runs before the lab is up |
| `ztp_disabled_001` | After up, each node with ZTP off is running and has none of its model's config mounts |
| `ztp_disabled_002` | After up, each node with ZTP off does not take the lab owner's SSH login, for the reason its model has once it has booted (below). The case waits for the booted state and fails if a login ever succeeds or the container stops |
| `config_delivery_001` | After up, each node with ZTP on is running, has every config mount of its model and takes the lab owner's SSH login by key through the lab's SSH config; then, once the `redeployed` fixture has redeployed one node of each setting, the redeployed node with ZTP on does the same |
| `ztp_disabled_003` | The redeployed node with ZTP off is as it was after up: running, with no config mount and no owner login |
| `ztp_disabled_004` | Once the nodes with ZTP on take the owner's login, and 20 s later, every node, the ones with ZTP off and the application container included, is running and was never restarted |

Before the lab is up, it also holds the command cases of
[`tests/specs/manifests.md`](../../specs/manifests.md), which need no lab of their own. Each works
in its own folder of the run, beside the lab's:

| Case | Check |
|---|---|
| `manifest_file_001` | `sherpa validate` in a folder with no `manifest.toml` exits 1 and says the file was not found |
| `manifest_file_009` | `sherpa validate` of a manifest with an unknown key on a node exits 1 and prints the file, the line and column, the key and the keys a node takes |
| `manifest_file_010` | `sherpa new` writes two Linux nodes and a link between them under a generated name, and `sherpa validate` passes it |
| `manifest_file_011` | `sherpa new` leaves an existing `manifest.toml` as it is and says so, and `sherpa new --force` writes over it |
| `validate_command_001` | `sherpa validate` of the lab's manifest, with an empty home and no `SHERPA_` variables, ticks every check, prints `Manifest validation passed!` and exits 0 |
| `validate_command_002` | `sherpa validate` of a 1001-character description exits 1 with the description check's message, and prints no check after it |
| `validate_command_005` | `sherpa up` of a node whose `environment_variables` key starts with a digit, which only the server checks, exits 1 with the server's message naming the key and the node; should up ever pass it, the case destroys what it built |

`config_delivery_001` comes before `ztp_disabled_003` because it brings the redeploy in after its
own after-up checks, and the `ztp_disabled` cases before it see every node as up left it.

Why a node with ZTP off refuses the owner's login, once it has booted:

- `srl02` and `ceos02` have no management address: SR Linux and cEOS as shipped never configure
  the address their container was given, so the SSH connection finds no route and the address
  does not answer a ping. While they boot the container still holds the address, and SR Linux's
  SSH server refuses the login there for about a minute; on cEOS nothing listens yet;
- `frr02` answers a ping on its address and refuses the SSH connection, because nothing
  installed an SSH server.

The fixture's teardown checks only that `sherpa destroy` succeeds and the lab is gone. That
destroy leaves no container, Docker network or host interface of a container lab is
`link_cleanup_008`, shown by [`p2p-containers`](../p2p-containers/README.md).
