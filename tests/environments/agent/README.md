# Agent worker lab

A long-lived worker VM for one more guest agent on the [agent bus](../../../docs/architecture/AGENT_BUS.md).
Each worker is its own lab with one `ubuntu_linux` node; this one is lab `sherpa-agent-2`, node
`agent-2`, and the guest agent in it is named `agent-2`. The [dev server](../dev/README.md) is
left as it is and keeps its own guest agent, `dev`. The bus stays on the host.

A worker is not a Sherpa server: it has the toolchains, Claude Code, the `nats` CLI, the GitHub
token and the checkouts, and no `sherpad`. Work that needs a Sherpa server, Docker or libvirt
(lab tests, integration tests, installs) goes to `dev`; formatting, clippy, unit and database
tests, code and docs go to either.

## Node

| Setting | Value |
|---|---|
| Image | `ubuntu_linux` `24.04-20260926`, the same as dev (see [ubuntu-2404](../ubuntu-2404/README.md)) |
| vCPUs | 8, as 1 socket × 4 cores × 2 threads, `host-passthrough` |
| Host CPUs | vCPUs on sibling pairs 1/17 to 4/20; QEMU and disk I/O threads on 0/16 |
| Memory | 24 GiB, preferred on NUMA node 0 |
| Disk | 150 GiB full copy, virtio, `cache = "none"`, `io = "native"` |
| Network | 4 virtio-net queues |

The sizing comes from sharing the host with dev. The host has 32 threads in two NUMA nodes:
node 0 is CPUs 0–7 and 16–23, node 1 is CPUs 8–15 and 24–31. Dev takes all of node 1 (14 vCPUs
and 2 threads for QEMU and disk I/O) and 48 GiB of its memory. The worker takes half of node 0,
so it never contends with dev for cores, caches or memory bandwidth, and leaves CPUs 5–7 and
21–23 to the host: `sherpad`, libvirt, Docker and the bus container, and the host agent. A guest
agent builds and runs clippy and unit tests, which 8 vCPUs and 24 GiB do comfortably; it does not
run nested labs, so it needs neither dev's memory nor its disk. 150 GiB holds the toolchains,
the checkouts and a few `target/` directories.

Memory prefers node 0, where the host's own processes also live
(`numa_memory_mode = "preferred"`). When node 0 is full it falls back to node 1 rather than the
host OOM killer ending the VM (#514), at the cost of slower remote memory. `numactl -H` on the
host shows whether node 0 has 24 GiB free. On another host, change `performance_tuning` to match
`lscpu -e`.

## Provisioning

There is one copy of the provisioning, dev's. `provision` here is a symbolic link to
`../dev/provision`, and the manifest names the same files dev does, plus `provision/worker.env`.
A manifest cannot point at `../dev/provision` directly and still be brought up from a lab
directory elsewhere: the CLI resolves relative paths from the directory `sherpa up` runs in, so
the link is what both layouts share.

`provision/worker.env` holds `SHERPA_DEV_SERVER=no`. It lands in `~/.env.d/` like the token,
and `provision/setup.sh` then stops after the shared steps of
[dev's provisioning](../dev/README.md#provisioning): apt packages, the mise tools in
`provision/mise.toml` (Rust, `gh`, Claude Code, the `nats` CLI and the rest), git set up with
`GH_TOKEN` through `gh`, the `bwks/sherpa` and `bwks/sherpa-docs` checkouts in `~/code/rust/`,
and `mkdocs`. sherpa-docs is cloned because a change to user-facing behaviour must update it in
the same piece of work. No Sherpa server is installed, initialized or logged in to, and no `dev`
OS account is made.

The GitHub token works as on dev: `provision/github.env` holds only `GH_TOKEN=$GH_TOKEN`, the
CLI fills it in from your environment when it provisions, and it lands in `~/.env.d/github.env`
in the guest, mode `600`.

## Bringing it up

On the host, from a lab directory outside the repository, linking the manifest and the
provisioning from the host's checkout (`<repo>` below):

```
mkdir -p ~/labs/sherpa-agent-2
cd ~/labs/sherpa-agent-2
ln -s <repo>/tests/environments/agent/manifest.toml manifest.toml
ln -s <repo>/tests/environments/agent/provision provision
GH_TOKEN=... sherpa up
```

Put the token in the environment the same way as for dev, for example with a `mise` task that
loads it from the secret store, rather than typing it. As with dev, do not interrupt `sherpa up`:
an interrupted `up` leaves a half-created lab that must be destroyed first. `sherpa up` writes
`.sherpa/lab-info.toml`, `.sherpa/ssh/config` and `.sherpa/ssh/key` into the lab directory,
outside the repository.

Connect with the lab's own SSH config, which goes through the host's Sherpa server:

```
cd ~/labs/sherpa-agent-2
ssh -F .sherpa/ssh/config agent-2.<lab id>
```

The SSH alias is `agent-2.<lab id>`, with `id` from `.sherpa/lab-info.toml`, not the bare node name.

The guest's address changes with every `sherpa up`; never write it down.

The checkout is cloned on its default branch. Until the agent bus is merged, switch it to the
branch that has `crates/agent-bus`, in the guest:

```
git -C ~/code/rust/sherpa switch feat/agent-bus
```

To tear down: push any work from the guest, then `sherpa destroy` in the lab directory.

## Starting its guest agent

With the bus running on dev's lab, on the host:

```
mise run agent-bus:agent ~/labs/sherpa-agent-2 agent-2
```

The `agent-bus:agent` task (see "Operating it" in the
[agent bus document](../../../docs/architecture/AGENT_BUS.md#operating-it)) runs
`~/.local/bin/agent-bus-start-agent` (installed from `dev/agent-bus/start-agent.sh`, next to the
`agent-bus` binary built in the dev guest; the host builds nothing) with the host's seed and the Claude token from the secret store.
It enrolls `agent-2`, giving it a fresh key and refusing any previous one, and starts the guest
agent over the lab's SSH config, alias `agent-2.<lab id>`, with `AGENT_BUS_URL` set to the URL in
the host's `bus.toml`, because the bus does not listen on this guest's default gateway (see
below). No key is made by hand and nothing is added to the agents file; `agent-2` must not be
listed there.

The bus's address is `ipv4_gateway` in dev's `.sherpa/lab-info.toml`. It changes when dev's lab is
brought up again; then run `bus start` and the same command again. Its log is
`~/agent-bus-agent-2.log` in the guest. Tasks go to it with
`agent-bus send --to agent-2 "<task>"`. To retire it, stop the agent and run
`agent-bus unenroll agent-2` on the host.

## Reaching the bus

The bus listens on one address: dev's management gateway, the host's address on dev's lab
bridge. This guest's default gateway is the host's address on its own bridge, where nothing
listens on 4222. The guest agent connects to dev's gateway address instead; the packets go to
the host through this lab's bridge, and because the address belongs to the host they are
delivered locally, not forwarded between the bridges.

**Checked:** from `agent-2` in lab `sherpa-agent-2`, on its own bridge, the bus at dev's gateway
address on port 4222 answered with NATS's `INFO` line, with no firewall change. One bus on the
host serves guests in other labs.

To recheck, with the bus running, in the guest:

```
timeout 5 bash -c 'exec 3<>/dev/tcp/<bus address>/4222; head -c 4 <&3; echo'
```

It prints `INFO` when the bus is reachable. Then, with the guest agent started, run
`agent-bus send --to agent-2 "Reply with exactly: ready."` on the host: the listener prints a
`completed` message from `agent-2` with the text `ready`. If the connection is refused or times
out, the host's firewall has changed and drops input from this bridge to port 4222; it then needs
a rule for this bridge, or the bus needs to listen on more than one address (see "One lab per
bus" in the agent bus document).

## Another worker

A worker for `agent-3` is a copy of this directory with the names changed. Copy it with the
link kept as a link (`cp -a tests/environments/agent tests/environments/agent-3`), then change:

| Where | From | To |
|---|---|---|
| `manifest.toml`, `name` | `sherpa-agent-2` | `sherpa-agent-3` |
| `manifest.toml`, `[[nodes]]` `name` | `agent-2` | `agent-3` |
| `manifest.toml`, `[nodes.performance_tuning]` | CPUs 1–4/17–20, QEMU on 0/16, node 0 | CPUs no other lab uses (below) |
| This README | `sherpa-agent-2`, `agent-2` | `sherpa-agent-3`, `agent-3` |
| The lab directory on the host | `~/labs/sherpa-agent-2` | `~/labs/sherpa-agent-3` |
| The start command | `mise run agent-bus:agent ~/labs/sherpa-agent-2 agent-2` | `mise run agent-bus:agent ~/labs/sherpa-agent-3 agent-3` |

The CPUs cannot just be copied: two labs pinned to the same cores contend for them. This host
has no free set left for a third worker of this size, as CPUs 5–7 and 21–23 are the host's. A
third worker means smaller workers, sharing cores on purpose, or another host; decide that
before copying. The node's memory should also fit on the NUMA node it prefers.
