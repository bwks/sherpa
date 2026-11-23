# Tailnet lab test

Tests a lab's Tailscale gateway on a real tailnet, end to end through the `sherpa` CLI: it joins
the tailnet, nodes answer from it, down takes it off, resume brings it back, and destroy and
`sherpa server clean` remove it. It holds the e2e real-tailnet cases of
[`tests/specs/tailnet.md`](../../specs/tailnet.md); what they need is at the end of that spec.

| Lab | Nodes | Role |
|---|---|---|
| `tna-<run>` | `vm01` (`alpine_linux`, the default version), `ctr01` (`generic` container, `alpine` 3.22) | `[tailscale]` enabled: the lab every case uses |
| `tnb-<run>` | `ctr01` | no `[tailscale]`: a second lab its gateway must not open (`tailnet_routes_003`) |
| `tnc-<run>` | `ctr01` | `[tailscale]` enabled: routes on the tailnet the first gateway must not take (`tailnet_routes_005`), removed with `sherpa server clean` (`tailnet_removal_011`) |
| `tnd-<run>` | `ctr01` | `[tailscale]` with a key the tailnet refuses (`tailnet_join_004`) |

The runner writes each manifest for the run itself, under a unique lab name, with
`auth_key_env = "SHERPA_TEST_TAILSCALE_AUTH_KEY"`; there is no manifest file here. The second and
third labs are removed before the cases after them, so a Docker restart never stops a gateway the
run cannot log out.

## The probe

The second tailnet device is a probe the runner makes: a `tailscale/tailscale` container of the
gateway's image (`v1.102.3`, the server's default), in a Docker network of its own, enrolled with
the same key and `--accept-routes`. Its firewall rejects every private range and the server's
address unless the packet leaves through `tailscale0`, so a lab answers it only through the
tailnet.

- A node answers when the probe reads its SSH banner (`SSH-2.0-`) with `nc`, or, for the
  container, which runs no SSH server, when it answers the probe's ping.
- A device joined or left the tailnet when the probe's `tailscale status --json` peer list holds
  it, or no longer holds its node ID. The gateway's host name and node ID are read from the
  gateway itself. Nothing reads or changes the tailnet's other devices or routes.
- Every wait for the tailnet polls, with a deadline of 3 minutes.

At the end every lab is destroyed, the probe logs out and is removed, and a second throwaway
probe checks that no gateway or probe device of the run is in its peer list. The run fails if a
Docker container, volume or network it made is left.

## What it checks

| Case | Check |
|---|---|
| `tailnet_join_001` | `sherpa up` prints the Tailscale table, connected with a `100.64.0.0/10` address, and the gateway holds the third address of the management subnet |
| `tailnet_join_002` | The gateway serves the lab's routes to the probe, and `vm01` answers SSH from it |
| `tailnet_join_009` | `vm01` answers SSH and `ctr01` answers ping from the probe |
| `tailnet_join_010` | `vm01` answers SSH on its IPv4 and its IPv6 management address |
| `tailnet_routes_004` | While the probe holds a connection to `vm01`, `vm01` sees it from the gateway's address, and its route to the probe's tailnet address is not through the gateway |
| `tailnet_status_012` | `sherpa inspect` prints the table: connected, approved, the lab's two subnets, a tailnet address |
| `tailnet_key_010` | The key is not in the server's database, lab folders, log files or sherpad's journal since the run started, the gateway's `docker inspect`, its processes' command lines or its `/run`, or the client's lab folder. Needs passwordless sudo, and is skipped without it |
| `tailnet_join_004` | `sherpa up` with a well-formed key the tailnet refuses fails, and leaves no gateway, identity volume or lab |
| `tailnet_routes_003` | The second lab's container (IPv4 and IPv6), the server's address on that lab's network, and the server's own address answer neither ping nor SSH from the probe, while `vm01` still does |
| `tailnet_routes_005` | Once the third lab's routes reach the probe, the first gateway has none of the routes on offer through `tailscale0`, and no DNS from the tailnet in `/etc/resolv.conf` |
| `tailnet_removal_011` | `sherpa server clean`, run by the admin, removes the third lab's gateway, volume and device, and the lab |
| `tailnet_suspend_011` | `sherpa down --node ctr01` and `sherpa resume --node ctr01`, without the key, leave the gateway running, connected and the same device |
| `tailnet_suspend_012` | `sherpa redeploy --node ctr01` without the key does too |
| `tailnet_suspend_025` | After `sherpa down --node` on both nodes, `sherpa resume` without the key starts them and keeps the device |
| `tailnet_suspend_014` | The web UI's start (`POST /labs/<id>/start` with the CLI's token as the `sherpa_auth` cookie) resumes the lab after `down --node` on both nodes, keeping the device; after `sherpa down` it is refused, saying to run `sherpa resume` in the lab folder, and starts nothing |
| `tailnet_suspend_001` | `sherpa down` stops the gateway, its device leaves the peer list, and the nodes no longer answer |
| `tailnet_suspend_002` | `sherpa resume` with the key prints the gateway connected, and `vm01` answers again |
| `tailnet_suspend_013` | A resume with a key the tailnet refuses fails, leaving the nodes and the gateway stopped; the next resume with the key connects it and `vm01` answers |
| `tailnet_suspend_019` | With `--restart-docker` only: after `sudo systemctl restart docker`, `sherpa resume` brings the gateway back as a new device, and its old device leaves the tailnet. The probe logs out before the restart and a new one joins after it |
| `tailnet_removal_001` | `sherpa destroy` removes the gateway, its identity volume and its device |

`tailnet_status_011` has no test: it needs routes approved by hand, which needs a Tailscale API
credential.

## Running it

The runner is a pytest module; `uv run` installs the pytest version pinned in its script header
and runs pytest on it, passing on any other arguments. The key is read from
`SHERPA_TEST_TAILSCALE_AUTH_KEY`; without it the runner says so and exits 2 before making
anything. See [Live tailnet tests](../../README.md#live-tailnet-tests) for running it with the key
from a pass-cli env file.

```sh
uv run tests/labs/tailnet/tailnet_lab_test.py --admin-user sherpa-admin
uv run tests/labs/tailnet/tailnet_lab_test.py --admin-user sherpa-admin --restart-docker
uv run tests/labs/tailnet/tailnet_lab_test.py -k tailnet_suspend_
uv run tests/labs/tailnet/tailnet_lab_test.py --self-test   # runner unit tests, seconds
```

It needs:

- to run on the Sherpa server, as a user in the `docker` group, with `ssh`;
- the tailnet set up as [the spec](../../specs/tailnet.md#cases-that-need-a-real-tailnet) says:
  the key, `autoApprovers` and access rules for the server's management ranges;
- the server with TLS on, and the `sherpa` CLI logged in over `wss://`, with a device password;
- the `alpine_linux` and `generic` container `alpine` 3.22 images; the probe image is pulled if
  it is missing;
- for `tailnet_removal_011`, an admin who does not own the labs, with `--admin-user` (or
  `SHERPA_LAB_TEST_ADMIN_USER`) and its password in `SHERPA_LAB_TEST_ADMIN_PASSWORD`; without it
  the case is skipped;
- for `tailnet_key_010`, passwordless sudo, which only reads; for `tailnet_suspend_019`,
  passwordless sudo and `--restart-docker`, which restarts Docker and so stops every container on
  the host until it is started again.

`--keep-failed` leaves the labs and probes of a failed run in place, devices included.
Each run logs every command and its output under `.tmp/lab-tests/tailnet-<id>/`, with the key,
tokens and passwords masked. A run takes about 5–10 minutes.
