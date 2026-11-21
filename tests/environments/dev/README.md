# Sherpa dev servers

Two long-lived, identical Sherpa servers for development, the two nodes `dev01` and `dev02` of
one lab, `sherpa-dev`, built by the host's Sherpa from this directory. Each is a full Sherpa
server, with `sherpad`, Docker and libvirt, toolchains and checkouts, and runs one guest agent on
the [agent bus](../../../docs/architecture/AGENT_BUS.md), named as its node. Any task can go to
either. Unlike the other environments they are not disposable: Sherpa is developed, installed and
tested inside them with root, without `sudo` on the host.

| | dev01 | dev02 |
|---|---|---|
| Node and guest agent | `dev01` | `dev02` |
| NUMA node | 1 | 0 |
| vCPUs on host CPUs | sibling pairs 10/26 to 15/31 | sibling pairs 2/18 to 7/23 |
| QEMU and disk I/O threads | 9/25 | 1/17 |
| Left to the host | 8/24 | 0/16 |

The two `[[nodes]]` entries in `manifest.toml` differ only in those rows; both use the same
provisioning files in `provision/`.

## Nodes

| Setting | Value |
|---|---|
| Image | `ubuntu_linux` `24.04-20260926` (see [ubuntu-2404](../ubuntu-2404/README.md)) |
| Login | the lab owner's own account, with their device password, see [Login account](#login-account) |
| vCPUs | 12, as 1 socket × 6 cores × 2 threads, `host-passthrough` |
| Memory | 32 GiB, preferred on the VM's NUMA node |
| Disk | 300 GiB full copy, virtio, `cache = "none"`, `io = "native"` |
| Network | 4 virtio-net queues |

## Login account

Each node's login is the account of the host Sherpa user who brings the lab up (the lab owner),
with that user's SSH keys and device password, and not `sherpa`. Its name is
that Sherpa user's name: nothing in the repository names it. It has passwordless `sudo` and is in
the `sherpa`, `docker`, `libvirt` and `kvm` groups, so the integration tests and the lab tooling
run as it. Sherpa's SSH config for the lab, `sherpa ssh`, the SSH provisioner, the agent-bus
tasks and `seed-images.sh` all follow it.

`sherpa` in the guest is the server's service account, as on any Sherpa install: a system uid,
shell `nologin`, no home, no `sudo` and no SSH key. It owns `/opt/sherpa` and runs `sherpad`.
Sherpa makes no `sherpa` login on a node, so the installer creates the real one. A guest built by
an older Sherpa has a first-boot `sherpa` login (uid 1000, with a home and `sudo`); setup deletes
it before the install.

The lab must be brought up by a Sherpa user whose name can be a login on lab nodes. A reserved
name such as `admin` cannot start labs, and a user named `dev` would clash with the guest's own
jump account (see step 14 below). Setup refuses a login named `dev` or `sherpa` before it changes
anything.

## NUMA layout

The host has 32 threads in two NUMA nodes: node 0 is CPUs 0–7 and 16–23, node 1 is CPUs 8–15
and 24–31, where CPU `n` and `n + 16` are the two threads of one core. Each dev server takes one
NUMA node: 12 vCPUs on six whole cores, and one more core for QEMU's emulator and disk I/O threads.
The node's first core stays with the host, for `sherpad`, libvirt, Docker, the bus container and
the host agent. The two nodes never share a core, a cache or a memory controller.

Memory prefers the VM's own node (`numa.memory_mode = "preferred"`) and falls back to the other
node when it is full, rather than the host OOM killer ending the VM (#514), at the cost of slower
remote memory. `numactl -H` on the host shows each node's free memory. Other VMs on the host,
such as the release test environments, are not pinned and can take memory on either node. On another host,
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
8. before the server is installed, while `/opt/sherpa` does not exist, deletes the first-boot
   `sherpa` account (uid 1000) that a guest built by an older Sherpa has, with its home and its
   line in `/etc/sudoers.d/90-cloud-init-users`, checking the changed file with `visudo` before it
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

The manifest names two env files for each node. Each line of the form `KEY=$NAME` is filled in by the CLI from
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

## Bringing the lab up

On the host, bring the lab up from a lab directory outside the repository, linking the manifest
and the provisioning from the host's checkout (`<repo>` below):

```
mkdir -p ~/labs/sherpa-dev
cd ~/labs/sherpa-dev
ln -s <repo>/tests/environments/dev/manifest.toml manifest.toml
ln -s <repo>/tests/environments/dev/provision provision
sherpa server user device-password
sherpa up
```

`sherpa up` is refused until the lab owner has set a device password, the password of their login
on the node. `sherpa server user device-password` asks for the Sherpa login password, then the
device password twice; set it once, not for each lab. Run `sherpa up` with the variables from
[Secrets](#secrets) in its environment. It builds and provisions both nodes. A manifest cannot
point at another directory's provisioning and still be brought up elsewhere: the CLI resolves
relative paths from the directory `sherpa up` runs in, so the links are what every layout
shares.

`sherpa up` prints nothing until it finishes when run through a `mise` task, because the task's
output is buffered. Do not stop it: an interrupted `up` leaves a half-created lab that must be
destroyed first. It writes `.sherpa/lab-info.toml`, `.sherpa/ssh/config` and
`.sherpa/ssh/known_hosts` into the lab directory, and no key: the node takes your own.

## Connecting

The guests are rebuilt from the manifest, so their addresses change with every `sherpa up`: never
write it down or hard-code it anywhere, and derive it from the files Sherpa generates for the lab.

`.sherpa/ssh/config` routes through a ProxyJump to the host's Sherpa server. Its alias is
`<node>.<lab id>`, with `id` from `.sherpa/lab-info.toml`:

```
cd ~/labs/sherpa-dev
ssh -F .sherpa/ssh/config dev01.<lab id>
```

For a faster connection, after every `sherpa up` or `sherpa redeploy` write a direct config
from it:

```
uv run <repo>/tests/environments/dev/direct_ssh_config.py ~/labs/sherpa-dev
ssh -F ~/labs/sherpa-dev/.sherpa/ssh/direct_config dev01
ssh -F ~/labs/sherpa-dev/.sherpa/ssh/direct_config dev02
```

`direct_ssh_config.py` resolves both nodes from the lab's `.sherpa/ssh/config` and
`.sherpa/lab-info.toml`, and writes `.sherpa/ssh/direct_config` with no ProxyJump, the bare node
name as its alias, and a shared master connection, so each command after the first takes
milliseconds. The master socket lives in `$XDG_RUNTIME_DIR`. `sherpa destroy` deletes the file
with `.sherpa/`. Close a node's master connection with `ssh -F .sherpa/ssh/direct_config -O exit dev01`.

The node logs in as you, with your own key: Sherpa's config names no key file for it, and
neither does the direct config, so `ssh` uses your SSH agent or your default key files, as it
already does for the ProxyJump hop. `ssh -F` does not read `~/.ssh/config`.

Do not add SSH config to `~/.ssh` or mount the guest on the host; use the generated config.

## Seeding images

Node images are not copied while the lab is built. Once provisioning has finished, copy the
host's VM images into each node, on the host:

```
<repo>/tests/environments/dev/seed-images.sh ~/labs/sherpa-dev dev01
<repo>/tests/environments/dev/seed-images.sh ~/labs/sherpa-dev dev02
```

For each model and version in [`images.toml`](images.toml), the script asks the guest's server
whether it has the image and skips it if so. Otherwise it copies the entry's `source` under
`/opt/sherpa/images` from the host with rsync over the lab's `.sherpa/ssh/config` into
`/var/tmp/sherpa-seed/` in the guest, imports it with the guest's `sherpa server image import`,
and deletes the staged copy. An interrupted copy resumes on the
next run, and running it again only imports what is missing. An image the host does not have or
that fails to import is reported, the rest go on, and the script exits with status 1. The
guest's CLI is already logged in, so no password is needed. The first version imported for a
model becomes its default, so `images.toml` lists the default first.

`images.toml` names one version of every VM model the host has, about 117 GiB in all, of which
`cisco_ise` is 23 GiB. With the toolchains and checkouts that fills about half of a guest's
300 GiB disk. An import briefly needs the image twice, the staged copy and the imported one.
Unikernel images (`nanos_unikernel`, `unikraft_unikernel`) are not seeded: their folders hold
`disk.qcow2` or `kernel.elf`, not a `virtioa.qcow2`.

The network-OS and Windows images are seeded for image and config work only. Never boot them in
a dev server: a dev server is a VM, so they run nested there and take many minutes to boot. Test
generic paths on Alpine, Ubuntu and containers. Windows checks are run by the host on its own
Windows box ([dev-windows](../dev-windows/README.md)).

Some images need a file that an import does not bring: vEOS boots from the Aboot ISO,
`/opt/sherpa/images/arista_eos/aboot_iso/aboot.iso` in the guest, and without it `sherpa up`
fails for a VM `arista_eos` node. Each `[[images]]` entry in `images.toml` has a `model`, a
`version` and a `source`, the disk's path under `/opt/sherpa/images` on the host. The source is
its own key because the host runs a released Sherpa whose folder names can differ from the
guest's model names. The sources follow the host's current folder names, which since its 0.7
update match the models; when an update renames the host's folders, update the sources.
`images.toml` lists the extra files under `[[extra_files]]`, each a `path` under
`/opt/sherpa/images` in the guest and a `source` under `/opt/sherpa/images` on the host. After
the images, the script compares each file's SHA-256 on the host and in the guest and skips it
when they match. Otherwise it copies the file through the same staging folder, checks the
copy's SHA-256, and installs it at its `path` in the guest, owned by the guest's `sherpa`
account and mode `644`, creating its folders as `sherpa`. Files are reported and
counted like the images: missing on the host, a failed copy or a checksum mismatch is a failure.

Container images are not seeded: the host's `/opt/sherpa/images` holds no container image to
copy, and `sherpa server image import` takes a container image only as a `docker save` archive.
Pull the ones a test needs in the guest, for example
`sherpa server image pull --model frr_linux --version 10.5.2`. Images that are not in a public
registry, such as `arista_eos` and `juniper_evolved` containers, need their vendor's archive
copied into the guest and imported with `sherpa server image import --model <model> --kind
container --version <version> --src <archive>`.

## Developing

Everything runs in the guest, as its login (see [Login account](#login-account)), which has
passwordless `sudo`. The tools from mise are on the `PATH` of login shells, so run commands
through one: `ssh -F .sherpa/ssh/direct_config dev01 'bash -lc "cargo --version"'`.

- **Code:** the checkouts are `~/code/rust/sherpa` and `~/code/rust/sherpa-docs`. They are the
  only working copies: commit and push from there. Git authenticates through `gh` with
  `GH_TOKEN`, which is read-only in a login shell; a push needs the write token (see
  [Secrets](#secrets)).
- **Build and install:** install a debug build. `dev/rebuild` builds the eBPF program, the
  Tailwind CSS and the workspace in debug mode, installs `sherpad` and `sherpa` in
  `/opt/sherpa/bin` and restarts `sherpad`:
  ```
  cd ~/code/rust/sherpa
  dev/rebuild
  ```
  A release build takes 6–7 minutes against a debug build's incremental seconds, and no live
  test on the dev server needs one: do not build `--release` to install. The VM code suite
  builds its own release archives, and `scripts/sherpa_update.sh` installs published ones.
- **Rebuilt artifacts:** `dev/rebuild` rewrites two committed files,
  `crates/ebpf-redirect/ebpf-redirect.elf` and `crates/server/web/static/css/tailwind.css`.
  Do not commit them from a rebuild: put them back with
  `git restore crates/ebpf-redirect/ebpf-redirect.elf crates/server/web/static/css/tailwind.css`.
  Commit them only with a change to the eBPF program, or to the templates' Tailwind classes.
- **Update to a release:** `sudo bash ~/code/rust/sherpa/scripts/sherpa_update.sh --version vX.Y.Z`.
- **Tests:** formatting, clippy, unit and database tests, integration tests and the lab tests all
  run in the guest, as described in [running tests](../../README.md). The guest's `sherpa` CLI
  is already logged in as `dev`.
- **Pre-commit hook:** enable it once in each checkout with
  `git config core.hooksPath .githooks`. It runs the `dev/check-*.sh` scripts, which refuse
  lint exceptions, code in a `mod.rs`, code blocks in doc comments, issue and pull request
  numbers in comments, and web UI template tags that decide from an enum's text; CI runs the
  same checks. The list is in the [main README](../../../README.md#code-quality).

Nested lab VMs run slower in the guest than on the host, so run the final check on the host
before a release.

## Guest agents

Each dev server runs one guest agent, named as its node: `dev01` on `dev01` and `dev02` on
`dev02`. With the bus running, start or restart one from the host with the `agent-bus:agent`
task described in "Operating it" in the
[agent bus document](../../../docs/architecture/AGENT_BUS.md#operating-it), giving the lab
directory and the node; the agent's name, an optional third argument, defaults to the node's:

```
mise run agent-bus:agent ~/labs/sherpa-dev dev01
mise run agent-bus:agent ~/labs/sherpa-dev dev02
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

The bus is started with `--lab` on this lab and listens on one address: the lab's management
gateway (`ipv4_gateway` in `.sherpa/lab-info.toml`), the host's address on the lab's bridge.
Both nodes are on that management network, so the gateway is each guest's default gateway and
both agents reach the bus there. The address changes only when the lab is brought up again;
then run `bus start` and start both agents again. `sherpa redeploy` keeps the lab and its
bridge, so the address stays.

The setup script runs `sherpad init` with `SHERPA_MANAGEMENT_PREFIX_IPV4=172.30.0.0/16` and
`SHERPA_MANAGEMENT_PREFIX_IPV6=fd00:b00a::/48`, so each dev server's lab management ranges are
outside the host's defaults, `172.31.0.0/16` and `fd00:b00b::/48`. A lab inside a dev server with
the defaults could be given the same subnet as `sherpa-dev` on the host, putting the bus address
on a bridge inside that dev server. Its agent would then reach its own bridge, get "connection
refused" and stop. `init` writes the prefixes only with a new config and keeps an existing one,
so a dev server whose config was written before needs `management_prefix_ipv4` and
`management_prefix_ipv6` set in `/opt/sherpa/config/sherpa.toml` and a `sherpad` restart.

## Tearing down

`sherpa destroy` takes both nodes down: push any work from both guests first, since destroying
the lab deletes their checkouts. Then, in the lab directory:

```
ssh -F .sherpa/ssh/direct_config -O exit dev01
ssh -F .sherpa/ssh/direct_config -O exit dev02
sherpa destroy
```

To rebuild one node and leave the other running, use `sherpa redeploy -n <node>` instead (see
[Rebuilding](#rebuilding)).

## Rebuilding

A guest built before the login moved off `sherpa` cannot be converted: the login is recorded
for the node only by `sherpa up` and `sherpa redeploy`, which both make a new disk, and setup
refuses to run there (`sherpa provision` stops at step 1).

Rebuild one node with `sherpa redeploy -n dev02` (or `dev01`) in the lab directory: it destroys
that node and recreates it with a new disk, runs its provisioners again, and leaves the other
node, the lab and the bus address as they are, so the other guest keeps working. `sherpa destroy`
and `sherpa up` rebuild both, and move the bus address.

Rebuilding a node deletes its disk. Copy out first what is worth keeping:

- unpushed commits and uncommitted work in both checkouts: push them, or copy patches out;
- the guest agent's `~/.claude` (memory and session history). Claude Code names its project
  folders from the checkout's path, so the memory goes back under
  `~/.claude/projects/-home-<login>-code-rust-sherpa` in the new guest;
- `~/.env.d/`: the files setup and the provisioner write there are made again, so only copy
  anything added by hand. Do not copy `sherpa-account.env` back: setup makes new passwords for
  the new server;
- scratch work in the home directory.

Then redeploy the node, or bring the lab up as in [Bringing the lab up](#bringing-the-lab-up),
write the direct config again, [seed the images](#seeding-images) of each rebuilt node and start
its guest agent. When the whole lab is rebuilt, the bus address moves: run `bus start` and start
both agents again (see [Reaching the bus](#reaching-the-bus)).

## Another dev server

This host has no cores left for a third dev server of this size: each NUMA node holds one, and
the host keeps one core per node. A third means smaller servers, sharing cores on purpose, or
another host; decide that first. A third is another `[[nodes]]` entry in `manifest.toml`, a
copy of one of the two with its own name and its own CPU, NUMA and disk I/O pinning (`cpu`,
`numa`, `boot_disk`), and its memory should fit on the NUMA node it prefers. Its agent takes
the node's name.
