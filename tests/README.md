# Running tests

This is the single reference for running Sherpa's tests. Other documents link here
instead of repeating these instructions.

## Layout

| Path | Contents |
|---|---|
| `crates/*/tests/`, `#[cfg(test)]` modules | Rust unit and integration tests (Cargo requires these locations) |
| `tests/specs/` | Test specs: the behaviours each area must show and the rules for tests ([README](specs/README.md)) |
| `tests/environments/` | Disposable Ubuntu VMs for the VM suites, the long-lived dev lab with its two dev servers, and the manifest of the Windows box on the Sherpa host that the client's Windows code is tested on ([README](environments/README.md)) |
| `tests/harness/` | VM test tooling: `test_env.py`, the release-test runner, guest helpers and the build cache |
| `tests/labs/` | Labs under test, driven by the `sherpa` CLI against an installed server ([README](labs/README.md)) |
| `tests/bats/` | Supplementary installer BATS tests |
| `tests/web/` | Web UI JavaScript tests |

## Quick reference

| Test | Command | Needs | Time |
|---|---|---|---|
| Format and lint | `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings` | Rust toolchain | about 1–5 min |
| Unit and database tests | `cargo test --workspace` or `cargo nextest run --workspace` | Nothing external (nextest from `mise install`) | about 1 min once built |
| Other crate integration | see [Crate integration tests](#crate-integration-tests) | Docker, libvirt, sudo | minutes |
| VM code suite | `uv run tests/harness/test_env.py run [ubuntu-2404] [ubuntu-2604]` | Sherpa host, logged-in CLI | about 40 min per release, in parallel when the server has the memory |
| Installer suite | `./tests/harness/test_install.sh --config tests/environments/<env>/release-test.toml` | Sherpa host, logged-in CLI | about 20–30 min |
| Private bridge lab test | `uv run tests/labs/private-bridge/private_bridge_lab_test.py` | Sherpa host with the alpine and FRR images, logged-in CLI | about 5–10 min |
| P2p containers lab test | `uv run tests/labs/p2p-containers/p2p_containers_lab_test.py` | On the Sherpa server: the node images (`arista_eos` container included, which is not in a public registry), `alpine:3.22`, Docker access, logged-in CLI | about 2–5 min |
| Container ZTP lab test | `uv run tests/labs/container-ztp/container_ztp_lab_test.py` | On the Sherpa server: the node images, Docker access, logged-in CLI, internet for the FRR node's packages | about 5–15 min |
| Manifest commands test | `uv run tests/labs/manifest-commands/manifest_commands_lab_test.py` | The `sherpa` CLI, logged in with an SSH key and a device password for the refused `sherpa up`; brings no lab up | seconds |
| Account lockout test | `uv run tests/labs/account-lockout/account_lockout_lab_test.py`, with `--stop-server` for `user_login_033` | The `sherpa` CLI, logged in as an admin, which creates and deletes the run's throwaway users; brings no lab up. `--stop-server` runs on the server host with passwordless sudo, and stops and starts sherpad | seconds |
| Cisco IOL lab test | `uv run tests/labs/cisco-iol/cisco_iol_lab_test.py` | On the Sherpa server: the `cisco_iol` image built with docker-cisco-iol, `alpine:3.22`, Docker access, logged-in CLI | about 3–5 min |
| NixOS lab test | `uv run tests/labs/nixos-linux/nixos_linux_lab_test.py` | The `nixos_linux` 26.05 image built from bwks/nixos-image, logged-in CLI with an SSH key, `ssh` | about 5 min |
| P2p links lab test | `uv run tests/labs/p2p-links/p2p_links_lab_test.py` | On the Sherpa server: the node images, `nicolaka/netshoot:v0.14`, Docker access, logged-in CLI | about 15–20 min |
| SSH access lab test | `uv run tests/labs/ssh-access/ssh_access_lab_test.py` | A route to the lab's management network (the Sherpa server), `ss`, the `alpine_linux` image, logged-in CLI; for `--ssh-jump`, a host account named as the Sherpa user | about 2 min |
| Owner login lab test | `uv run tests/labs/owner-login/owner_login_lab_test.py` | A route to the lab's management network (the Sherpa server), a CLI logged in as an admin, which creates and deletes the run's admin and owner, the `ubuntu_linux` 24.04 and `nokia_srlinux` 26.7.2 images | about 5 min |
| Console lab test | `uv run tests/labs/console/console_lab_test.py --admin-user NAME` | Logged-in CLI with an SSH key; an admin, such as `admin`, its password in `SHERPA_LAB_TEST_ADMIN_PASSWORD`, who creates and deletes the run's other user; the `alpine_linux` and `generic` container `alpine` 3.22 images | about 1–5 min |
| Talos lab test | `uv run tests/labs/talos-linux/talos_lab_test.py` | Sherpa host with the Talos, Ubuntu, Keycloak and Vault images, logged-in CLI, internet | about 25–35 min |
| Web UI JavaScript | `node tests/web/test_theme.cjs` | Node.js | seconds |
| Harness self-tests | `uv run tests/harness/<script>.py --self-test` | Python 3.15+ | seconds |

Nothing runs these automatically. Every suite is started by hand, and the VM suites work
from a snapshot of the checkout, so you can keep editing and committing while they run.

The Python scripts run on Python 3.15 or later, which uv installs. Start them with `uv run`, as
above, so they never use a system Python, whether or not mise is active. Each script that is
run directly starts with `#!/usr/bin/env -S uv run --script` and an inline metadata block
(PEP 723) that requires Python 3.15 and no dependencies, so `./tests/harness/test_env.py` works
too for the executable ones. Because of that block, `uv run` takes the Python requirement from
the script, not from `pyproject.toml`, and runs it in its own environment in uv's cache; inside
the checkout it still reads the `[tool.uv]` settings there, which allow only uv-managed Python.
Sibling modules (`vm_release_test.py`, `build_cache.py`, `host_memory.py`) are imported from the script's
directory, as before. `test_env.py --self-test` fails if a script in `tests/` or `dev/` has any
other `#!` line.
Nothing in the harness needs Python on the Sherpa host. The guest helpers,
`tests/harness/*_guest.py`, run on Python 3.15 from uv as well: before it runs them, the harness
installs a pinned uv release on each test VM, checked against its published sha256
(`UV_VERSION` and `UV_SHA256` in `tests/harness/vm_release_test.py`), and uv's Python 3.15 in
`/opt/uv/python`. The VM's system Python is never used, and the run fails if uv or Python 3.15
cannot be installed.

## While iterating

Check and test only the crate you change, with a test-name filter where it helps, and run the
workspace commands below once before a commit. Times after a one-line edit in `shared`, on a
12-core guest (medians, sccache off):

| Command | Time |
|---|---|
| `cargo test -p shared --lib --no-run` | 8 s |
| `cargo check -p sherpad` | 20 s |
| `cargo check --workspace --all-targets` | 28 s |
| `cargo clippy --workspace --all-targets -- -D warnings` | 42 s |
| `cargo build --workspace --all-targets` | 50 s |

The first `-p` build of a crate resolves features for that crate alone and builds its own copy
of some dependencies: 160–270 s, once.

Add `-- --nocapture` to see what the tests print.

## Before a commit

Once before each commit, not after each edit, run format, lint and the
[unit tests](#unit-tests) (`cargo test --workspace`), once each, in that order. Formatting and
linting must pass first; CI fails on either:

```sh
cargo fmt
cargo clippy --workspace --all-targets -- -D warnings
```

## Where to build and test

- Development, builds, installs and tests can run in the dev servers, nodes `dev01` and `dev02`
  of the Sherpa lab `sherpa-dev`, with root and the toolchains. How to bring them up, connect to
  them and work in them is in [`environments/dev/README.md`](environments/dev/README.md). Follow
  it; do not hard-code their addresses or add host-side SSH config or mounts.
- Dev server installs are debug builds with `dev/rebuild`. Never build `--release` for a live
  test: a release build takes 6–7 minutes.
- Never boot network-OS VMs (vEOS, vSRX, vEvolved, CSR, Cat8000/9000, IOSv, IOS XR, NX-OS, ASA,
  FTD, PAN-OS, AOS-CX, ISE, RouterOS, SONiC VM and the like), Windows or cJunosEvolved in the dev
  servers. They are nested there and too slow. Test generic paths on Alpine, Ubuntu and
  containers; a change to one of those models' configs is covered by template unit tests, not a
  live boot.
- Windows checks are run by the host, on a bare-metal Windows VM on the Sherpa host; agents
  don't use a Windows VM. Only some changes need one: changes to the interactive terminal
  (Jato's console code, `sherpa console`, `sherpa ssh`) and the installer. CI covers the rest.
  For one, cross-compile the client (`cargo build -p sherpa --target x86_64-pc-windows-gnu`),
  leave `sherpa.exe` in `/var/tmp`, and say so in your report.

## Unit tests

```sh
cargo test --workspace
```

Unit tests need no Docker, libvirt or network access. The database tests run here too:
sherpad embeds its database, and each test opens its own in-memory database
(`db::open_in_memory`). Integration tests are marked `#[ignore]` and are skipped here.

[cargo-nextest](https://nexte.st/) runs the same tests, each in its own process, and reports
the time of each test. `mise install` installs the version pinned in `mise.toml`; CI runs it
through `cargo llvm-cov nextest`. Use it locally the same way as `cargo test`:

```sh
cargo nextest run --workspace
cargo nextest run -p sherpad guest_account
```

nextest does not run doctests; the workspace has none (doc comments carry no code blocks).

## Crate integration tests

Integration tests are `#[ignore]` tests that talk to real services. They need a host with
the services each crate uses; the [VM code suite](#vm-code-suite) provides one
automatically. The db crate has none: its tests use in-memory databases and run with the unit
tests. Run the crates you need:

| Crate | Command | Needs |
|---|---|---|
| container | `cargo test -p container -- --ignored --test-threads=1` | Docker, `alpine:latest`; the IOL test also needs a `localrepo/cisco_iol` image built with docker-cisco-iol, and passes without running when there is none |
| network | `sudo -E env PATH="$HOME/.cargo/bin:$PATH" cargo test -p network -- --ignored --test-threads=1` | root (rtnetlink) |
| sherpa (client) | `cargo test -p sherpa -- --ignored` | No route to TEST-NET-1 (`192.0.2.0/24`) other than a default route; takes about 10 s |
| libvirt | `cargo test -p libvirt -- --ignored --test-threads=1` | libvirtd, `sherpa-pool` |
| sherpad | `sudo -E env PATH="$HOME/.cargo/bin:$PATH" cargo test -p sherpad -- --ignored --test-threads=1` | root, Docker, libvirt, test images |

sherpad's integration tests are one test binary, `integration`
(`crates/server/tests/integration/`), with a module per area: `account_lockout`, `auth`, `image_management`,
`lab_authorization`, `lab_lifecycle`, `security`, `user_management` and `websocket`. Run one
area by its module path, and run root tests from the built binary so root never writes to
`target/`:

```sh
cargo test -p sherpad --test integration lab_lifecycle::
cargo test -p sherpad --test integration --no-run --message-format=json   # prints the binary
sudo -n <binary> --ignored --test-threads=1 lab_lifecycle::
```

`tests/harness/run-integration-tests.sh` runs all of them in order. Container, network,
libvirt and sherpad tests create real resources: run them only on a disposable host.

Opt-in live tests read these environment variables:

| Variable | Effect |
|---|---|
| `SHERPA_OVERLAY_LIVE=1` | Runs the libvirt overlay isolation test (disposable volumes in `sherpa-pool`), as root: it checks the guest-visible blocks with `qemu-io` on the volume files, which only QEMU can read. The VM code suite sets it and runs the integration tests as root |
| `SHERPA_TEST_KEEP_FAILED_LABS=1` | A failed lab test keeps its lab for debugging |
| `SHERPA_TEST_TAILSCALE_*` | The live tailnet enrollment test, which joins a real tailnet; see [Live tailnet tests](#live-tailnet-tests). The VM code suite sets them itself when it has a key |

Run the overlay test from the built libvirt test binary:

```sh
sudo -n env SHERPA_OVERLAY_LIVE=1 <binary> --ignored --test-threads=1 live_overlay
```

### Live tailnet tests

The gateway daemon tests need Docker with TUN and nftables, and no tailnet. Each creates and
removes its own network, gateway and volume, and pulls the gateway image if it is absent:

```sh
cargo test -p container -- --ignored tailnet_routes_002 tailnet_key_009 tailnet_suspend_010
```

The enrollment tests join a real tailnet. Use a disposable test tailnet and a disposable Docker
network, and set in the shell that runs them:

| Variable | Value |
|---|---|
| `SHERPA_TEST_TAILSCALE_AUTH_KEY` | Auth key for the test tailnet, from a secret manager |
| `SHERPA_TEST_TAILSCALE_NETWORK` | The existing Docker network the gateway joins |
| `SHERPA_TEST_TAILSCALE_IPV4` | An unused gateway IPv4 address in that network |
| `SHERPA_TEST_TAILSCALE_IPV6` | Optional: an unused gateway IPv6 address in that network |
| `SHERPA_TEST_TAILSCALE_ROUTES` | Comma-separated management prefixes to advertise |
| `SHERPA_TEST_SERVER_CONFIG` | Optional, for these and the daemon tests: a complete server `sherpa.toml` whose `[tailscale]` settings replace the defaults |

```sh
cargo test -p container -- --ignored tailnet_join_003 tailnet_suspend_003
```

They take turns on the one gateway address and each removes its gateway and identity volume;
the Docker network is left. The VM code suite creates the network and sets these itself (see
[VM code suite](#vm-code-suite)). What the other tailnet cases need is in
[the tailnet spec](specs/tailnet.md#cases-that-need-a-real-tailnet).

### After a crashed run

The tests remove their own leftovers before each run, but a crash can leave resources behind.
Their names start with `sherpa-test-` (Docker containers and networks, libvirt networks and
pools) or `st-` (network interfaces). On the disposable host, list them and remove each:

```sh
docker ps -a --format '{{.Names}}' | grep '^sherpa-test-' | xargs -r docker rm -f
docker network ls --format '{{.Name}}' | grep '^sherpa-test-' | xargs -r docker network rm
virsh -c qemu:///system net-list --all --name | grep '^sherpa-test-'   # net-destroy, then net-undefine
virsh -c qemu:///system pool-list --all --name | grep '^sherpa-test-'  # pool-destroy, then pool-undefine
ip -br link show | grep '^st-'                                         # sudo ip link del <name>
```


## VM code suite

Builds the checkout and tests it in fresh Ubuntu VMs created on a Sherpa server:

```sh
uv run tests/harness/test_env.py run                  # 24.04 and 26.04
uv run tests/harness/test_env.py run ubuntu-2604      # one release
```

It needs an authenticated `sherpa` CLI with at least four hours left on its token
(`sherpa login`), a device password set for that user (`sherpa server user device-password`;
`sherpa up` is refused without one, and the run stops early), and a Sherpa server with the
environment images. For each release it:

1. builds `sherpad` and `sherpa` release archives from a snapshot of the checkout,
   including uncommitted changes,
2. creates a fresh lab, installs those archives with the checkout's installer, initializes
   the server and checks API and CLI access,
3. installs the Rust toolchain, then runs the workspace unit tests and the db, container,
   network, libvirt and sherpad integration tests inside the VM. The tests are built once, for
   the whole workspace, and each integration crate's test executables are run from that build:
   `cargo test -p <crate>` would resolve dependency features for that crate alone and rebuild
   them (about 8 minutes for `db`),
4. fails on any failed test, a VM test on a guest without `/dev/kvm` included, and on Docker,
   libvirt or network resources left behind.

The live tailnet enrollment test joins a real tailnet, so it runs only when the run starts
with an auth key in `SHERPA_TEST_TAILSCALE_AUTH_KEY` on the host; without one it is skipped and
`test-results.toml` records why. Use an ephemeral key, from a secret manager:

```sh
SHERPA_TEST_TAILSCALE_AUTH_KEY=<key> uv run tests/harness/test_env.py run
```

The key reaches the guest in its run settings, like the build cache password, never on a
command line, and is redacted from the logs. The guest creates a disposable Docker network for
the test's gateway and removes it before checking for leftover resources. The gateway joins the
tailnet as `tntest<pid>`, advertises `172.31.250.0/24` and logs out when the test stops it.

### Memory

Each release's VM asks the server for its `memory` (30 GiB in both environments). The releases
start together, and their `sherpa up` calls take turns. When the server refuses one for memory
(the CLI exits 75 for the server's `InsufficientMemory` error), nothing was created: the runner records
`failure_kind = "memory-refused"` with the server's message, keeps nothing and exits 75 itself.
`test_env.py` then waits for the other releases to finish and runs the refused one on its own,
then the next refused one. A release refused while it runs alone fails. A failed release keeps
its VM, and with it its memory, so remove it (`--cleanup-run`, below) before running again. The
runner recognises a refusal by `sherpa up`'s exit code, never its output, so the host's `sherpa`
must be a build that has the exit code; an older one exits 1 and the refusal is a failure.

When a run fails after its VM exists, the runner asks the host whether the VM stopped, with
`virsh domstate --reason <domain>`. If it did, it reads the kernel's `oom-kill:` lines as the
SSH user, without `sudo`: `journalctl --dmesg --boot --grep=oom-kill:` (which needs the `adm` or
`systemd-journal` group), then `dmesg` (which needs `kernel.dmesg_restrict=0`). A line whose
`task_memcg` is the domain's libvirt scope, `machine-qemu\x2d<id>\x2d<domain>.scope`, proves the
kill. When neither log is readable, a domain libvirt saw crash (`shut off (crashed)`) while the
`oom_kill` count in `/sys/fs/cgroup/machine.slice/memory.events` rose since the VM started counts
as killed, and the error says the kernel log could not be read. The run then reports
`<domain> was killed by the host OOM killer: <evidence>` instead of the step's `exited 255`,
with `failure_kind = "oom-killed"` and `guest_loss_evidence` in `result.toml`. A VM that stopped
without that evidence is `failure_kind = "domain-stopped"`. `tests/harness/host_memory.py` holds
the parsing and its tests.

A passing VM is destroyed; a failing one is kept. Then:

```sh
uv run tests/harness/test_env.py rerun .tmp/vm-release-tests/<run-id>    # retest a kept VM
uv run tests/harness/vm_release_test.py --cleanup-run .tmp/vm-release-tests/<run-id>   # remove it
```

A rerun uploads the current checkout and repeats the test steps without reinstalling; it is
for debugging, not a substitute for a fresh run. Results are in `.tmp/test-env-runs/<id>/`
(build logs, candidate hashes) and `.tmp/vm-release-tests/<run-id>/` (`result.toml`,
`test-results.toml`, `logs/`). Settings are in each environment's `release-test.toml`.

To manage one environment by hand: `uv run tests/harness/test_env.py up | check | destroy <env>`.

### Build cache

The test build is the suite's longest step. A Redis server on the Sherpa host can keep the
compiled crates between runs: sccache in each guest stores what it compiles there and reuses it
on the next run, from either release.

```sh
uv run tests/harness/test_env.py cache start    # start Redis in a container on this host
uv run tests/harness/test_env.py cache status   # running state and the number of entries
uv run tests/harness/test_env.py cache clear    # empty the cache
uv run tests/harness/test_env.py cache stop     # remove the container; the data is kept
```

- Redis listens on the Sherpa server's address from the CLI configuration, where the guests
  reach the host, and requires a password. The password is generated on the first start.
  It and the cached data are kept in `~/.local/state/sherpa-tests/build-cache/`, outside the
  checkout, and the password reaches each guest in its run settings, never on a command line.
- Memory is capped and the least recently used entries are evicted; the image, port and
  limit are in `tests/environments/build-cache.toml`, with the pinned sccache release.
- Runs use the cache whenever that directory holds a password. A guest that cannot reach
  Redis builds uncached rather than failing, since sccache would fail every compilation.
- `test-results.toml` records the hits and misses in `[build_cache]`, and the summary prints
  them. Crates whose source changed since the cache last saw them miss; linking and build
  scripts are never cached. The guests build into the same target directory path every run:
  crates that generate code embed that path, so a per-run path would change their keys and
  those of every crate depending on them.

## Installer suite

Tests the checkout's installer and uninstaller with published release binaries:

```sh
./tests/harness/test_install.sh --config tests/environments/ubuntu-2604/release-test.toml
```

It needs curl, OpenSSH with scp and an authenticated `sherpa` CLI on the machine that runs it.
Its SSH account on the Sherpa host must be able to read the base image's checksum and query
domains with `virsh`. The guest needs internet access for Ubuntu packages, Docker, container
registries and the pinned uv release the harness installs.

The runner creates and destroys its own uniquely named lab from the environment's manifest,
never one brought up by hand, with a generated guest name and credentials for each run.
Without `--config` it uses `tests/environments/ubuntu-2604/release-test.toml`, which holds the
connection settings (an empty `server_url` uses the CLI's server), the base image's identity,
the candidate, timeouts, output and retention; the guest's resources and Ubuntu release are in
the environment's `manifest.toml`.

The harness creates its lab as the logged-in CLI's user, so that user needs a device password
(`sherpa server user device-password`) like every lab owner; the runner stops with that message
when it is not set. The upgrade scenario also checks the refusal and the fix on the guest.

The guest helper reports each case a step shows by its ID, with `PASS` or `FAIL`, and the step
fails naming every case that failed. A failed or timed-out required check fails the run with a
nonzero exit, and no required check is skipped silently.

### Candidates

By default a run tests the checkout's installer, update and uninstall scripts with the published
release named in `[candidate] version`. That proves the scripts, not unpublished source in the
binaries. To test locally built archives:

```sh
./tests/harness/test_install.sh --version v0.6.0 --artifact-dir /path/to/release-artifacts
```

The directory holds `sherpad-x86_64-unknown-linux-gnu.tar.gz` and
`sherpa-x86_64-unknown-linux-gnu.tar.gz`, each with its binary at the archive's root. The
installer reads them from `SHERPA_ARTIFACT_DIR` instead of downloading them, and the runner
checks the installed bytes against the archives and the installed version. Each run records
the checkout's commit and whether it is dirty, the script, archive and binary hashes, the base
image's checksum and the VM's identity. `--result-file <path>` also writes a nonsecret TOML
reference to the run's receipt, naming that run when the output directory holds earlier ones.

### Scenarios

Select one with `--scenario`:

- `lifecycle` (default): preflight errors, a fresh install, ownership and permissions, Docker,
  libvirt and the database, `sherpad init`, API and CLI access with TLS checked, reboot
  recovery, a repeat install, uninstall with `--keep-data` and reinstall, `--remove-data`,
  `--remove-all` and a repeat uninstall. Each uninstall compares packages, users and groups,
  Docker images, volumes and unrelated containers, and libvirt domains, networks and pools
  before and after, and keeps those inventories with the result; the server's journal and
  log are captured before it. `--keep-data` keeps the database, its key, the configuration,
  the TLS certificate and the CLI; `--remove-data` empties the database and keeps the rest;
  `--remove-all` removes `/opt/sherpa` and the binary symlinks. Every mode keeps the Docker
  and libvirt packages, images, users, groups and libvirt definitions; only a fresh guest is a
  fresh baseline.
- `preflight`: only the baseline and the argument and error checks.
- `upgrade`: installs a baseline release with that release's own installer
  (`git show <baseline>:scripts/sherpa_install.sh`), initializes it and adds a non-admin user
  through the API, then upgrades the same installation with the checkout's `sherpa_update.sh`
  and the candidate archives, without reinitializing. It checks the binaries, API and CLI
  logins, both users' records, credentials and privileges, the configuration, the environment
  file's assignments and the server's TLS key, again after a reboot, and records
  them in `upgrade-state.toml`; the database key is kept too when the baseline has one (from
  0.6.4). A last `device-password` step checks that the upgraded user, who never set one, is
  refused by `sherpa up` and is not once the harness sets a generated device password through
  the API.
- `failure-checks`: a timed-out command, a SIGTERM while a guest command runs and incomplete
  provisioning after a readiness timeout, each in its own fresh guest. Each must give a failed
  receipt and a nonzero exit, keep the resource identity and diagnostics, stop its guest
  command and pass targeted cleanup. These expected faults pass the failure suite, never a
  lifecycle run.

```sh
./tests/harness/test_install.sh --scenario upgrade --baseline-version v0.5.35 --version v0.6.0 --artifact-dir /path/to/release-artifacts
./tests/harness/test_install.sh --scenario failure-checks --version v0.6.0 --artifact-dir /path/to/release-artifacts
```

The upgrade scenario downloads the baseline release on this machine, or takes it from
`--baseline-artifact-dir`; the optional `[baseline]` table of `release-test.toml` has
`version` and `artifact_directory`, and the flags override it. The candidate must be local
archives (`--artifact-dir`), both releases 0.5.0 or later, and they must differ in version and
in bytes; anything else is refused before a guest is made. A server on a release before 0.5.0
updates to 0.5.x first.

### Results and cleanup

Each run writes `result.toml`, its inputs and numbered logs to a private directory under
`.tmp/vm-release-tests/`, with generated credentials redacted, and prints its path. A passing
run destroys its own lab after checking the lab's and the domain's identity; a failed run is
kept, and `--keep-vm` keeps a passing one too. To look at a kept run, use `sherpa inspect` in
its result directory and `sherpa ssh` with the receipt's `node_name`. Remove it with
`--cleanup-run`, which checks its identity, also works on a partly provisioned run and records
the removal in the receipt without changing its status:

```sh
./tests/harness/test_install.sh --cleanup-run /absolute/path/to/run-directory
```

After a passing lifecycle run, check the receipt against the archives and the checkout:

```sh
uv run tests/harness/verify_release_test.py --receipt /path/to/run/result.toml --artifact-dir /path/to/release-artifacts --version v0.6.0
```

`--result-file` takes the reference instead of `--receipt`. The verifier refuses changed
installer or harness scripts, changed archive or binary hashes, missing required steps,
incomplete resource evidence, failed cleanup and a dirty checkout; it compares the recorded
commit with `HEAD`, or with `--commit`. `--allow-dirty` allows a dirty checkout for local
development, and `--stage-evidence <dir>` copies only the receipt and the guest's resource
inventories, without connection settings, credentials or keys.

The BATS tests run one function of a script with its commands stubbed, as a user that is not root:

```sh
bats tests/bats/install_tests.bats
bats tests/bats/update_tests.bats
bats tests/bats/uninstall_tests.bats
```

## Lab tests

Lab tests build real labs with the `sherpa` CLI against the server it is logged in to, check
them from inside the lab, and always destroy them. They test whatever `sherpad` that server
runs, so install the build under test first.

```sh
uv run tests/labs/private-bridge/private_bridge_lab_test.py  # private bridges: a VM and containers
uv run tests/labs/p2p-containers/p2p_containers_lab_test.py  # P2p links on NOS, app and OS containers
uv run tests/labs/container-ztp/container_ztp_lab_test.py    # ZTP on and off on the mounted-config containers
uv run tests/labs/manifest-commands/manifest_commands_lab_test.py  # sherpa validate, sherpa new and a refused sherpa up; no lab
uv run tests/labs/p2p-links/p2p_links_lab_test.py            # P2p link requirements on every pair of node kinds
uv run tests/labs/cisco-iol/cisco_iol_lab_test.py            # Cisco IOL: config, SSH, links, TCP, write memory
uv run tests/labs/nixos-linux/nixos_linux_lab_test.py        # NixOS VMs: cloud-init, host keys, owner login, eth names, text files, provisioner
uv run tests/labs/console/console_lab_test.py --admin-user NAME  # sherpa console and sherpa vnc: prompt, escapes, in use, --force, other accounts, containers
uv run tests/labs/ssh-access/ssh_access_lab_test.py         # sherpa ssh and provisioners: direct, and --ssh-jump
uv run tests/labs/owner-login/owner_login_lab_test.py --admin-user NAME  # owner logins on nodes, admin refusals, down/resume and redeploy
uv run tests/labs/talos-linux/talos_lab_test.py         # Talos clusters: single and ha labs in parallel; ha with Keycloak login and Vault secrets
```

See [labs/README.md](labs/README.md) for the rules and each lab's README for what it checks.

The SSH access, owner login, container ZTP, manifest commands, console, P2p containers, NixOS and private bridge lab tests are pytest modules (see
`tests/specs/README.md`): their script headers pin pytest, so `uv run` runs pytest on them and
passes on pytest arguments such as `-k` and `--keep-failed`;
`uvx --python 3.15 pytest==9.2.0 tests/labs/ssh-access/ssh_access_lab_test.py` runs one directly.

## Harness self-tests

```sh
uv run tests/harness/test_env.py --self-test
uv run tests/harness/build_cache.py --self-test
uv run tests/harness/host_memory.py --self-test
uv run tests/harness/vm_release_test.py --self-test
uv run tests/harness/vm_release_guest.py --self-test
uv run tests/harness/vm_release_faults.py --self-test
uv run tests/harness/verify_release_test.py --self-test
uv run tests/labs/private-bridge/private_bridge_lab_test.py --self-test
uv run tests/labs/p2p-containers/p2p_containers_lab_test.py --self-test
uv run tests/labs/p2p-links/p2p_links_lab_test.py --self-test
uv run tests/labs/console/console_lab_test.py --self-test
uv run tests/labs/ssh-access/ssh_access_lab_test.py --self-test
uv run tests/labs/owner-login/owner_login_lab_test.py --self-test
uv run tests/labs/talos-linux/talos_lab_test.py --self-test
uv run tests/labs/cisco-iol/cisco_iol_lab_test.py --self-test
uv run tests/labs/container-ztp/container_ztp_lab_test.py --self-test
uv run tests/labs/manifest-commands/manifest_commands_lab_test.py --self-test
uv run tests/labs/nixos-linux/nixos_linux_lab_test.py --self-test
uv run tests/labs/labkit.py  # the helpers the lab runners share
```

Run these after changing anything in `tests/harness/` or a lab runner. They test the tooling, not
Sherpa, so they stay outside the behaviour IDs of `tests/specs/README.md`, and each keeps only
checks whose failure means a lab or VM run could pass when it should fail, or the tooling could
touch what it does not own on the host or leak a credential. The lab runners' `SelfTest` classes
set `__test__ = False` so pytest does not collect them as lab cases.

## When to run what

| Change | Run |
|---|---|
| Any change | Format, lint and unit tests |
| Database schema or queries | Also the db integration tests |
| Runtime code (server services, libvirt, container, network, guest templates) | The VM code suite on one release, in the background, before merging |
| Installer or uninstaller scripts | The installer suite |
| Lab networking (links, bridges, container or VM interfaces) | The private bridge lab test, against a server running the change; for P2p or container interfaces, also the P2p containers lab test |
| Talos nodes or clusters | The Talos lab test, against a server running the change |
| `sherpa console` or `sherpa ssh` terminal code | The console lab test |
| How `sherpa ssh` or provisioners connect to nodes (routes, jump, prompts) | The SSH access lab test, on the Sherpa server, and the client's ignored tests |
| Before a release, or periodically | The VM code suite on both releases |

These are guidelines, not gates: none of them blocks a commit, a merge or a release. Run the
"Before a release" row on the bump pull request, before clicking Release
(`docs/architecture/RELEASE.md`).

## Known issues

- **TAP devices on kernel 6.8+:** `create_tap()` uses `InfoKind::Tun` through rtnetlink,
  which kernel 6.8 rejects. `test_create_tap`, `test_get_ifindex` and
  `test_ebpf_redirect_between_taps` detect this and skip.

## Conventions

- Integration tests use `#[ignore]` and run with `-- --ignored`.
- Tests that share host state run with `--test-threads=1`.
- Database tests open their own in-memory database (`db::open_in_memory`).
- Container tests prefix resources with `sherpa-test-`.
- Tests that create labs always clean them up, including on failure.
