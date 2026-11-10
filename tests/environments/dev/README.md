# Sherpa dev servers

Two long-lived, identical Sherpa servers for development, each built as a lab by the host's
Sherpa: `dev` and `agent-2`. Each is a full Sherpa server, with `sherpad`, Docker and libvirt,
toolchains and checkouts, and runs one guest agent on the
[agent bus](../../../docs/architecture/AGENT_BUS.md). Any task can go to either. Unlike the
other environments they are not disposable: Sherpa is developed, installed and tested inside
them with root, without `sudo` on the host. The original design and its decisions are in
[the investigation note](../../../docs/investigations/sherpa-dev-server.md).

| | dev | agent-2 |
|---|---|---|
| Directory | `tests/environments/dev` | `tests/environments/agent` |
| Lab | `sherpa-dev` | `sherpa-agent-2` |
| Node and guest agent | `dev` | `agent-2` |
| NUMA node | 1 | 0 |
| vCPUs on host CPUs | sibling pairs 10/26 to 15/31 | sibling pairs 2/18 to 7/23 |
| QEMU and disk I/O threads | 9/25 | 1/17 |
| Left to the host | 8/24 | 0/16 |

The two manifests differ only in those rows. `tests/environments/agent/provision` is a link to
`../dev/provision`, so both use the same provisioning files: do not move or rename them without
updating the link.

## Node

| Setting | Value |
|---|---|
| Image | `ubuntu_linux` `24.04-20260926` (see [ubuntu-2404](../ubuntu-2404/README.md)) |
| vCPUs | 12, as 1 socket × 6 cores × 2 threads, `host-passthrough` |
| Memory | 32 GiB, preferred on the VM's NUMA node |
| Disk | 300 GiB full copy, virtio, `cache = "none"`, `io = "native"` |
| Network | 4 virtio-net queues |

## NUMA layout

The host has 32 threads in two NUMA nodes: node 0 is CPUs 0–7 and 16–23, node 1 is CPUs 8–15
and 24–31, where CPU `n` and `n + 16` are the two threads of one core. Each dev server takes one
node: 12 vCPUs on six whole cores, and one more core for QEMU's emulator and disk I/O threads.
The node's first core stays with the host, for `sherpad`, libvirt, Docker, the bus container and
the host agent. The two VMs never share a core, a cache or a memory controller.

Memory prefers the VM's own node (`numa_memory_mode = "preferred"`) and falls back to the other
node when it is full, rather than the host OOM killer ending the VM (#514), at the cost of slower
remote memory. `numactl -H` on the host shows each node's free memory. Other VMs on the host,
such as the release test environments, are not pinned and can take memory on either node; see
[the OOM investigation](../../../docs/investigations/host-memory-oom.md). On another host,
change `performance_tuning` to match `lscpu -e`.

## Provisioning

`provision/setup.sh` runs through the SSH provisioner as the node's login user, after the node
is up. It is safe to run again with `sherpa provision`. It:

1. installs system libraries and compilers with apt: `build-essential`, `libvirt-dev`, `clang`,
   `llvm` and others;
2. installs [mise](https://mise.jdx.dev) and the tools in `provision/mise.toml`, which the
   provisioner copies to `~/.config/mise/config.toml`: Rust stable with `rustfmt`, `clippy` and
   `llvm-tools`, nightly with `rust-src`, `bpf-linker` 0.10.2 for the eBPF program (0.12 needs a
   system LLVM that Ubuntu 24.04 does not ship), `gh`, `shellcheck`, `bats`, `uv`, `sccache`,
   Claude Code and the `nats` CLI. mise follows the repository's `rust-toolchain.toml` files, and
   login shells put its tools on the `PATH`;
3. sets up git to authenticate with `GH_TOKEN` (the read-only token) through `gh`, committing as
   the token's GitHub account with its no-reply address;
4. clones `bwks/sherpa` and `bwks/sherpa-docs` into `~/code/rust/`. When they already exist it
   only fetches, so work in progress is never touched;
5. installs `mkdocs` with uv on a uv-managed Python 3.15 (never the system Python), as a tool on
   the `PATH`, at the versions in sherpa-docs' `requirements.txt`, so `mkdocs build` and
   `mkdocs serve` work in `~/code/rust/sherpa-docs`;
6. installs the [beautiful-rust](https://github.com/morganlinton/BeautifulRust) Claude Code
   plugin (the `rust-best-practices`, `rust-review` and `rust-port` skills) for the guest
   agents, from a clone pinned to one commit in `~/.local/share/claude-marketplaces/`;
7. installs a Sherpa server with the checkout's `scripts/sherpa_install.sh`;
8. initializes the server unattended: it generates the `admin` password, a `dev` user (an
   administrator) with its own password, and an SSH key `~/.ssh/id_ed25519` for it, kept in
   `~/.env.d/sherpa-account.env` (mode `600`). It runs `sherpad init` with them and starts
   `sherpad`;
9. points the guest's `sherpa` CLI at its own server, trusting the server's certificate file,
   and logs in as `dev`;
10. creates a `dev` OS account with the same SSH key and trusts the guest's own host key. The SSH
   config Sherpa writes for each lab reaches its nodes through the server as the Sherpa user
   (`ProxyJump dev@<server>`), so lab tests and SSH provisioners need both, as on any Sherpa
   server.

Nothing is prompted for. Login shells load `~/.env.d/`, so `sherpa login` works there without
prompts too. Node images are not part of provisioning; [seed them](#seeding-images) afterwards.

## Secrets

The manifests name two env files. Each line of the form `KEY=$NAME` is filled in by the CLI from
your environment when it provisions, so no secret is in the repository, on a command line or sent
to the Sherpa server. They land in `~/.env.d/` in the guest, mode `600`. `sherpa up` stops before
it changes the node if a variable they name is not set.

- `provision/github.env`: `GH_TOKEN=$GH_TOKEN_RO`, the read-only GitHub token, for git and `gh`.
  It is the only GitHub token on the guest's disk. It can read issues, pull requests, checks and
  CI logs, and clone and fetch; GitHub refuses it a push or any other write.

  Make it a fine-grained personal access token for the account the guests commit as, with access
  to `bwks/sherpa` and `bwks/sherpa-docs` and these repository permissions, all read-only:
  Contents, Issues, Pull requests, Actions, Commit statuses and Metadata. Store it in the secret
  store as `GH_TOKEN_RO`.

  The write token, `GH_TOKEN` in the secret store, never goes to the guest's disk. The host sends
  it to the guest agent each time `agent-bus:agent` starts it, and the agent sets it only for
  `--github` tasks (see "GitHub tokens" in the
  [agent bus document](../../../docs/architecture/AGENT_BUS.md#github-tokens)). Pushing from a
  shell in the guest therefore fails: commit there, and push from a `--github` task.
- `provision/sccache.env`: `RUSTC_WRAPPER=sccache`, and `SCCACHE_REDIS_ENDPOINT` and
  `SCCACHE_REDIS_PASSWORD` from your environment. Rust builds in the guest keep their compiled
  crates in the host's build cache, the Redis server that
  `uv run tests/harness/test_env.py cache start` runs (see
  [build cache](../../README.md#build-cache)). The endpoint is
  `redis://<Sherpa server address>:6379`, with the address from the host's CLI configuration;
  the password is in `~/.local/state/sherpa-tests/build-cache/` on the host.

On its own, sccache refuses to start when it cannot reach Redis, and every compilation then
fails. `provision/sccache.env` therefore also sets `SCCACHE_MULTILEVEL_CHAIN=redis` and
`SCCACHE_MULTILEVEL_WRITE_ERROR_POLICY=ignore`. As a one-level multi-level cache, Redis is
best-effort: when it refuses connections (the cache is stopped), sccache starts anyway and
compiles locally, but each cacheable crate waits about 20 seconds for Redis first, about 10 on
the lookup and 10 on the store. A build of the workspace is then much slower than one without
the cache. When the host does not answer at all, the server's check of Redis outlasts the
client's 10 second wait for the server and the build still fails. In either case, start the
cache on the host, or build without it in the guest with `RUSTC_WRAPPER= cargo ...`.

Put the variables in the environment with a `mise` task that loads them from the secret store,
rather than typing them.

## Bringing a lab up

On the host, bring each lab up from a lab directory outside the repository, linking the manifest
and the provisioning from the host's checkout (`<repo>` below). For dev:

```
mkdir -p ~/labs/sherpa-dev
cd ~/labs/sherpa-dev
ln -s <repo>/tests/environments/dev/manifest.toml manifest.toml
ln -s <repo>/tests/environments/dev/provision provision
sherpa up
```

For agent-2, the same in `~/labs/sherpa-agent-2` with `tests/environments/agent`. Run `sherpa up`
with the variables from [Secrets](#secrets) in its environment. A manifest cannot point at
another directory's provisioning and still be brought up elsewhere: the CLI resolves relative
paths from the directory `sherpa up` runs in, so the links are what every layout shares.

`sherpa up` prints nothing until it finishes when run through a `mise` task, because the task's
output is buffered. Do not stop it: an interrupted `up` leaves a half-created lab that must be
destroyed first. It writes `.sherpa/lab-info.toml`, `.sherpa/ssh/config` and `.sherpa/ssh/key`
into the lab directory.

## Connecting

The guest is rebuilt from its manifest, so its address changes with every `sherpa up`: never
write it down or hard-code it anywhere, and derive it from the files Sherpa generates for the lab.

`.sherpa/ssh/config` routes through a ProxyJump to the host's Sherpa server. Its alias is
`<node>.<lab id>`, with `id` from `.sherpa/lab-info.toml`:

```
cd ~/labs/sherpa-dev
ssh -F .sherpa/ssh/config dev.<lab id>
```

For a faster connection, after every `sherpa up` write a direct config from it:

```
uv run <repo>/tests/environments/dev/direct_ssh_config.py ~/labs/sherpa-dev
ssh -F ~/labs/sherpa-dev/.sherpa/ssh/direct_config dev
```

`direct_ssh_config.py` resolves each node from the lab's `.sherpa/ssh/config` and
`.sherpa/lab-info.toml`, and writes `.sherpa/ssh/direct_config` with no ProxyJump, the bare node
name as its alias, and a shared master connection, so each command after the first takes
milliseconds. The master socket lives in `$XDG_RUNTIME_DIR`. `sherpa destroy` deletes the file
with `.sherpa/`. Close the master connection with `ssh -F .sherpa/ssh/direct_config -O exit dev`.

Do not add SSH config to `~/.ssh` or mount the guest on the host; use the generated config.

## Seeding images

Node images are not copied while the lab is built. Once provisioning has finished, copy the
host's VM images in, on the host:

```
<repo>/tests/environments/dev/seed-images.sh ~/labs/sherpa-dev dev
<repo>/tests/environments/dev/seed-images.sh ~/labs/sherpa-agent-2 agent-2
```

For each model and version in [`images.toml`](images.toml), the script asks the guest's server
whether it has the image and skips it if so. Otherwise it copies
`/opt/sherpa/images/<model>/<version>/virtioa.qcow2` from the host with rsync over the lab's
`.sherpa/ssh/config` into `/var/tmp/sherpa-seed/` in the guest, imports it with the guest's
`sherpa server image import`, and deletes the staged copy. An interrupted copy resumes on the
next run, and running it again only imports what is missing. An image the host does not have or
that fails to import is reported, the rest go on, and the script exits with status 1. The
guest's CLI is already logged in, so no password is needed. The first version imported for a
model becomes its default, so `images.toml` lists the default first.

Container images are not seeded: the host's `/opt/sherpa/images` holds no container image to
copy, and `sherpa server image import` takes a container image only as a `docker save` archive.
Pull the ones a test needs in the guest, for example
`sherpa server image pull --model frr_linux --version 10.5.2`. Images that are not in a public
registry, such as `arista_ceos` and `juniper_cevolved`, need their vendor's archive copied into
the guest and imported with `sherpa server image import --model <model> --version <version>
--src <archive>`.

## Developing

Everything runs in the guest, as its login user `sherpa`, which has passwordless `sudo`. The
tools from mise are on the `PATH` of login shells, so run commands through one:
`ssh -F .sherpa/ssh/direct_config dev 'bash -lc "cargo --version"'`.

- **Code:** the checkouts are `~/code/rust/sherpa` and `~/code/rust/sherpa-docs`. They are the
  only working copies: commit and push from there. Git authenticates through `gh` with
  `GH_TOKEN`, which is read-only in a login shell; a push needs the write token (see
  [Secrets](#secrets)).
- **Build and install:**
  ```
  cd ~/code/rust/sherpa
  cargo build --release -p sherpad
  sudo install -o sherpa -g sherpa -m 755 target/release/sherpad /opt/sherpa/bin/sherpad
  sudo systemctl restart sherpad
  ```
  `cargo build --release` at the workspace root does not rebuild `sherpad`.
- **Update to a release:** `sudo bash ~/code/rust/sherpa/scripts/sherpa_update.sh --version vX.Y.Z`.
- **Tests:** formatting, clippy, unit and database tests, integration tests and the lab tests all
  run in the guest, as described in [running tests](../../README.md). The guest's `sherpa` CLI
  is already logged in as `dev`.

Nested lab VMs run slower in the guest than on the host, so run the final check on the host
before a release.

## Guest agents

Each dev server runs one guest agent named after its node. With the bus running, start or
restart one from the host with the `agent-bus:agent` task described in "Operating it" in the
[agent bus document](../../../docs/architecture/AGENT_BUS.md#operating-it):

```
mise run agent-bus:agent ~/labs/sherpa-dev dev
mise run agent-bus:agent ~/labs/sherpa-agent-2 agent-2
```

Give the lab directory as an absolute path, since mise may run the task from another directory.
The task enrolls the agent with a fresh key, refusing any previous one, and pipes the key, the
Claude token, the GitHub write token and the bus's CA certificate over the lab's SSH config into
`dev/agent-bus/start-guest.sh` in the guest, with `AGENT_BUS_URL` set to the URL in the host's
`bus.toml`. No key is made or stored by hand, and neither agent is listed in the agents file. The
agent's log is `~/agent-bus-<node>.log` in the guest. To retire one, stop its agent and run
`agent-bus unenroll <node>` on the host.

### Reaching the bus

The bus is started with `--lab` on dev's lab and listens on one address: dev's management
gateway (`ipv4_gateway` in dev's `.sherpa/lab-info.toml`), the host's address on dev's lab
bridge. It changes when dev's lab is brought up again; then run `bus start` and start both
agents again. agent-2's own default gateway is the host's address on its own bridge, where
nothing listens on 4222, so its agent connects to dev's gateway address instead. The packets go
to the host through agent-2's bridge, and because the address belongs to the host they are
delivered locally, not forwarded between the bridges.

**Checked:** from `agent-2` in lab `sherpa-agent-2`, on its own bridge, the bus at dev's gateway
address on port 4222 answered with NATS's `INFO` line, with no firewall change.

To recheck, with the bus running, in agent-2:

```
timeout 5 bash -c 'exec 3<>/dev/tcp/<bus address>/4222; head -c 4 <&3; echo'
```

It prints `INFO` when the bus is reachable. If the connection is refused or times out, the
host's firewall drops input from that bridge to port 4222; it then needs a rule for the bridge,
or the bus needs to listen on more than one address (see "One lab per bus" in the agent bus
document).

## Tearing down

Push any work from the guest first: destroying a lab deletes its checkouts. Then, in its lab
directory:

```
ssh -F .sherpa/ssh/direct_config -O exit <node>
sherpa destroy
```

## Another dev server

This host has no cores left for a third dev server of this size: each NUMA node holds one, and
the host keeps one core per node. A third means smaller servers, sharing cores on purpose, or
another host; decide that before copying a directory. A copy (`cp -a`, which keeps the
`provision` link) changes the lab name, the node name and `performance_tuning`, and the node's
memory should fit on the NUMA node it prefers.
