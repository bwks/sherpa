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
| Node | `dev` | `agent-2` |
| Guest agent | `dev01` | `dev02` |
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
| Login | the lab owner's account (`provision_current_user = true`), see [Login account](#login-account) |
| vCPUs | 12, as 1 socket × 6 cores × 2 threads, `host-passthrough` |
| Memory | 32 GiB, preferred on the VM's NUMA node |
| Disk | 300 GiB full copy, virtio, `cache = "none"`, `io = "native"` |
| Network | 4 virtio-net queues |

## Login account

The manifests set `provision_current_user = true`, so the node's login is the account of the
host Sherpa user who brings the lab up, with that user's SSH key, and not `sherpa`. Its name is
that Sherpa user's name: nothing in the repository names it. It has passwordless `sudo` and is in
the `sherpa`, `docker`, `libvirt` and `kvm` groups, so the integration tests and the lab tooling
run as it. Sherpa's SSH config for the lab, `sherpa ssh`, the SSH provisioner, the agent-bus
tasks and `seed-images.sh` all follow it.

`sherpa` in the guest is the server's service account, as on any Sherpa install: a system uid,
shell `nologin`, no home, no `sudo` and no SSH key. It owns `/opt/sherpa` and runs `sherpad`.
cloud-init still makes a first-boot `sherpa` login on every node (uid 1000, with a home and
`sudo`); setup deletes it before the install so that the installer creates the real one.

The lab must be brought up by a Sherpa user whose name can be a login on lab nodes. A reserved
name such as `admin` gets no account and the node falls back to the `sherpa` login, and a user
named `dev` would clash with the guest's own jump account (see step 14 below). Setup refuses
both before it changes anything.

## NUMA layout

The host has 32 threads in two NUMA nodes: node 0 is CPUs 0–7 and 16–23, node 1 is CPUs 8–15
and 24–31, where CPU `n` and `n + 16` are the two threads of one core. Each dev server takes one
node: 12 vCPUs on six whole cores, and one more core for QEMU's emulator and disk I/O threads.
The node's first core stays with the host, for `sherpad`, libvirt, Docker, the bus container and
the host agent. The two VMs never share a core, a cache or a memory controller.

Memory prefers the VM's own node (`numa.memory_mode = "preferred"`) and falls back to the other
node when it is full, rather than the host OOM killer ending the VM (#514), at the cost of slower
remote memory. `numactl -H` on the host shows each node's free memory. Other VMs on the host,
such as the release test environments, are not pinned and can take memory on either node; see
[the OOM investigation](../../../docs/investigations/host-memory-oom.md). On another host,
change the `cpu`, `numa` and `boot_disk` pinning to match `lscpu -e`.

## Provisioning

`provision/setup.sh` runs through the SSH provisioner as the node's login, after the node is up.
It is safe to run again with `sherpa provision`. It:

1. stops before any change when the login is `sherpa` or `dev` (see
   [Login account](#login-account));
2. installs system libraries and compilers with apt: `build-essential`, `libvirt-dev`, `clang`,
   `llvm` and others;
3. installs [mise](https://mise.jdx.dev) and the tools in `provision/mise.toml`, which the
   provisioner copies to `~/.config/mise/config.toml`: Rust stable with `rustfmt`, `clippy` and
   `llvm-tools`, nightly with `rust-src`, `bpf-linker` 0.10.2 for the eBPF program (0.12 needs a
   system LLVM that Ubuntu 24.04 does not ship), `gh`, `shellcheck`, `bats`, `uv`, `sccache`,
   Claude Code and the `nats` CLI. mise follows the repository's `rust-toolchain.toml` files, and
   login shells put its tools on the `PATH`;
4. sets up git to authenticate with `GH_TOKEN` (the read-only token) through `gh`, committing as
   the token's GitHub account with its no-reply address;
5. clones `bwks/sherpa` and `bwks/sherpa-docs` into `~/code/rust/`. When they already exist it
   only fetches, so work in progress is never touched;
6. installs `zensical` with uv on a uv-managed Python 3.15 (never the system Python), as a tool
   on the `PATH`, at the version in sherpa-docs' `requirements.txt`, so `zensical build` and
   `zensical serve` work in `~/code/rust/sherpa-docs`;
7. installs the [beautiful-rust](https://github.com/morganlinton/BeautifulRust) Claude Code
   plugin (the `rust-best-practices`, `rust-review` and `rust-port` skills) for the guest
   agents, from a clone pinned to one commit in `~/.local/share/claude-marketplaces/`;
8. before the server is installed, while `/opt/sherpa` does not exist, deletes cloud-init's
   first-boot `sherpa` account (uid 1000) with its home and its line in
   `/etc/sudoers.d/90-cloud-init-users`, checking the changed file with `visudo` before it
   replaces the old one. It never touches an installed server;
9. installs a Sherpa server with the checkout's `scripts/sherpa_install.sh`, which creates the
   `sherpa` service account and adds the login to the `sherpa` group;
10. on every run, checks that `sherpa` is that service account: a system uid, shell `nologin`,
   not in `sudo`, no sudoers rule, no `authorized_keys`, and the owner of `/opt/sherpa`. If not,
   it stops and says what is wrong; it does not fix it;
11. adds the login to the `docker`, `libvirt` and `kvm` groups, from its next session on;
12. initializes the server unattended: it generates the `admin` password, a `dev` user (an
   administrator) with its own password, and an SSH key `~/.ssh/id_ed25519` for it, kept in
   `~/.env.d/sherpa-account.env` (mode `600`). It runs `sherpad init` as `sherpa` with them,
   giving it a copy of the public key outside the login's home, which `sherpa` cannot read, and
   starts `sherpad`;
13. copies the server's certificate from `/opt/sherpa/.certs` (readable only by `sherpa`) to
   `~/.sherpa/config/server.crt`, points the guest's `sherpa` CLI at its own server, trusting
   that copy, and logs in as `dev`;
14. creates a `dev` OS account with the same SSH key and trusts the guest's own host key. The SSH
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
  it to the guest agent each time `agent-bus:agent` starts it, and the agent uses it only for
  `--github` tasks and, after their runs, to update the pull requests' Generation section (see "GitHub tokens" in the
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

The node logs in as you, with your own key: Sherpa's config names no key file for it, and
neither does the direct config, so `ssh` uses your SSH agent or your default key files, as it
already does for the ProxyJump hop. `ssh -F` does not read `~/.ssh/config`.

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

Some images need a file that an import does not bring: vEOS boots from the Aboot ISO,
`/opt/sherpa/images/arista_veos/aboot_iso/aboot.iso`, and without it `sherpa up` fails for an
`arista_veos` node. `images.toml` lists such files under `[[extra_files]]`, each a `path` under
`/opt/sherpa/images`. After the images, the script compares each file's SHA-256 on the host and
in the guest and skips it when they match. Otherwise it copies the file through the same staging
folder, checks the copy's SHA-256, and installs it at the same path in the guest, owned by the
guest's `sherpa` account and mode `644`, creating its folders as `sherpa`. Files are reported and
counted like the images: missing on the host, a failed copy or a checksum mismatch is a failure.

Container images are not seeded: the host's `/opt/sherpa/images` holds no container image to
copy, and `sherpa server image import` takes a container image only as a `docker save` archive.
Pull the ones a test needs in the guest, for example
`sherpa server image pull --model frr_linux --version 10.5.2`. Images that are not in a public
registry, such as `arista_ceos` and `juniper_cevolved`, need their vendor's archive copied into
the guest and imported with `sherpa server image import --model <model> --version <version>
--src <archive>`.

## Developing

Everything runs in the guest, as its login (see [Login account](#login-account)), which has
passwordless `sudo`. The tools from mise are on the `PATH` of login shells, so run commands
through one: `ssh -F .sherpa/ssh/direct_config dev 'bash -lc "cargo --version"'`.

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
- **Pre-commit hook:** enable it once in each checkout with
  `git config core.hooksPath .githooks`. It runs `dev/check-lint-exceptions.sh`, which refuses
  new `allow`/`expect` lint exceptions; CI runs the same check.

Nested lab VMs run slower in the guest than on the host, so run the final check on the host
before a release.

## Guest agents

Each dev server runs one guest agent, named apart from its node: `dev01` on `dev` and `dev02` on
`agent-2`. With the bus running, start or restart one from the host with the `agent-bus:agent`
task described in "Operating it" in the
[agent bus document](../../../docs/architecture/AGENT_BUS.md#operating-it), giving the lab
directory, the node and the agent's name:

```
mise run agent-bus:agent ~/labs/sherpa-dev dev dev01
mise run agent-bus:agent ~/labs/sherpa-agent-2 agent-2 dev02
```

Give the lab directory as an absolute path, since mise may run the task from another directory.
The task enrolls the agent with a fresh key, refusing any previous one, and pipes the key, the
Claude token, the GitHub write token and the bus's CA certificate over the lab's SSH config into
`dev/agent-bus/start-guest.sh` in the guest, with `AGENT_BUS_URL` set to the URL in the host's
`bus.toml`. The script pipes the key and both tokens on to the agent's stdin: the agent holds them
in memory, never in its environment. No key is made or stored by hand, and neither agent is listed
in the agents file. The agent is built from the guest's own checkout, so a change to the agent or
to `start-guest.sh` reaches a guest when its checkout is pulled and its agent started again. The
agent's log is `~/agent-bus-<agent>.log` in the guest. To retire one, stop its agent and run
`agent-bus unenroll <agent>` on the host with the host's seed, which also deletes its inbox.
Starting a node under a new name stops the agent of its old name, as one agent runs per
checkout; the steps for a rename, and what happens to the old name's history, are under
"Renaming a guest agent" in the
[agent bus document](../../../docs/architecture/AGENT_BUS.md#operating-it).

### Reaching the bus

The bus is started with `--lab` on dev's lab and listens on one address: dev's management
gateway (`ipv4_gateway` in dev's `.sherpa/lab-info.toml`), the host's address on dev's lab
bridge. It changes when dev's lab is brought up again; then run `bus start` and start both
agents again. agent-2's own default gateway is the host's address on its own bridge, where
nothing listens on 4222, so its agent connects to dev's gateway address instead. The packets go
to the host through agent-2's bridge, and because the address belongs to the host they are
delivered locally, not forwarded between the bridges.

The setup script runs `sherpad init` with `SHERPA_MANAGEMENT_PREFIX_IPV4=172.30.0.0/16` and
`SHERPA_MANAGEMENT_PREFIX_IPV6=fd00:b00a::/48`, so each dev server's lab management ranges are
outside the host's defaults, `172.31.0.0/16` and `fd00:b00b::/48`. A lab inside a dev server with
the defaults could be given the same subnet as dev's lab on the host, putting the bus address on
a bridge inside that dev server. Its agent would then reach its own bridge, get "connection
refused" and stop. `init` writes the prefixes only with a new config and keeps an existing one,
so a dev server whose config was written before needs `management_prefix_ipv4` and
`management_prefix_ipv6` set in `/opt/sherpa/config/sherpa.toml` and a `sherpad` restart.

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

## Rebuilding

A guest built before the login moved off `sherpa` cannot be converted: the login is recorded
for the node only by `sherpa up` and `sherpa redeploy`, which both make a new disk, and setup
refuses to run there (`sherpa provision` stops at step 1). Rebuild it with `sherpa destroy` and
`sherpa up`, agent-2 first: the bus listens on dev's lab, so agent-2 can be rebuilt while dev
keeps working.

Destroying a lab deletes its disk. Copy out first what is worth keeping:

- unpushed commits and uncommitted work in both checkouts: push them, or copy patches out;
- the guest agent's `~/.claude` (memory and session history). Claude Code names its project
  folders from the checkout's path, so the memory goes back under
  `~/.claude/projects/-home-<login>-code-rust-sherpa` in the new guest;
- `~/.env.d/`: the files setup and the provisioner write there are made again, so only copy
  anything added by hand. Do not copy `sherpa-account.env` back: setup makes new passwords for
  the new server;
- scratch work in the home directory.

Then bring the lab up as in [Bringing a lab up](#bringing-a-lab-up), write the direct config,
[seed the images](#seeding-images) again and start the guest agent. When dev is rebuilt, the bus
address moves: run `bus start` and start both agents again (see [Reaching the bus](#reaching-the-bus)).

## Another dev server

This host has no cores left for a third dev server of this size: each NUMA node holds one, and
the host keeps one core per node. A third means smaller servers, sharing cores on purpose, or
another host; decide that before copying a directory. A copy (`cp -a`, which keeps the
`provision` link) changes the lab name, the node name and the CPU, NUMA and disk I/O pinning (`cpu`, `numa`, `boot_disk`), and the node's
memory should fit on the NUMA node it prefers.
